//! FETR v1 reading and chronological request selection for cache replays.

use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
};

use feuer_types::{ByteRange, ObjectKeyHash};
use rustc_hash::FxHashMap;

/// Errors from trace decoding, input files, or replay preparation.
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// One recorded request start or lookup outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    /// 0 start, 1 memory hit, 2 disk hit, 3 callback success, 4 callback error, 5 noncovering download.
    pub kind: u8,
    /// Identity joining this request's start and outcome within one trace session.
    pub request_id: u64,
    /// Nanoseconds since trace creation, not necessarily ordered in the package.
    pub time_ns: u64,
    /// The original object identity, already hashed by the writer.
    pub object_key: ObjectKeyHash,
    /// Exact requested range. `0..u64::MAX` denotes a whole-object request.
    pub requested: ByteRange,
    /// Callback's downloaded range, present only for kinds 3 and 5.
    pub downloaded: Option<ByteRange>,
}

/// Validates a package header and length, then decodes and validates each event.
/// Consuming the iterator validates the entire package, not trace completeness.
pub fn decode_package(data: &[u8]) -> Result<impl Iterator<Item = Result<Event>> + '_> {
    if data.get(..8) != Some(b"FETR\x01\0\0\0") {
        return Err("expected FETR v1 package header".into());
    }
    if data.len() == 8 || !(data.len() - 8).is_multiple_of(65) {
        return Err("expected nonempty 65-byte FETR events".into());
    }
    Ok(data[8..].as_chunks::<65>().0.iter().map(|record| {
        let u64_at = |offset| u64::from_le_bytes(record[offset..offset + 8].try_into().unwrap());
        let kind = record[0];
        if kind > 5 {
            return Err("unknown FETR event kind".into());
        }
        let requested = ByteRange::new(u64_at(33), u64_at(41))?;
        let downloaded = match kind {
            3 | 5 => Some(ByteRange::new(u64_at(49), u64_at(57))?),
            _ => {
                if u64_at(49) != 0 || u64_at(57) != 0 {
                    return Err("unexpected downloaded range for FETR event kind".into());
                }
                None
            }
        };
        if kind == 3 {
            let downloaded = downloaded.unwrap();
            if requested == ByteRange::new(0, u64::MAX).unwrap() {
                if downloaded.start() != 0 {
                    return Err("whole-object download must start at zero".into());
                }
            } else if !downloaded.contains(requested) {
                return Err("callback success does not cover requested range".into());
            }
        }
        if kind == 5 && downloaded.unwrap().contains(requested) {
            return Err("noncovering download covers requested range".into());
        }
        Ok(Event {
            kind,
            request_id: u64_at(1),
            time_ns: u64_at(9),
            object_key: ObjectKeyHash(u128::from_le_bytes(record[17..33].try_into().unwrap())),
            requested,
            downloaded,
        })
    }))
}

/// One arrival to replay with an exact range and the recorded object hash.
#[derive(Clone, Copy, Debug)]
pub struct Request {
    /// Identity used to break ties between equal arrival timestamps.
    pub request_id: u64,
    /// Nanoseconds relative to the earliest recorded request start.
    pub time_ns: u64,
    /// Recorded identity, never converted back into a string or rehashed.
    pub object_key: ObjectKeyHash,
    /// Exact requested byte range.
    pub range: ByteRange,
}

/// Selected request starts, with recorded outcomes reported separately from synthetic replay results.
pub struct TraceRequests {
    /// Requests in timestamp order, with request ID as the tie breaker.
    pub requests: Vec<Request>,
    /// Counts of recorded outcomes for selected starts, indexed by FETR kind.
    pub outcomes: [usize; 6],
    /// Selected starts without a recorded outcome. These are not established cache misses.
    pub incomplete: usize,
}

/// Reads a single `.feuer` package or a `.tar.zst` archive containing one trace session.
/// Selects all starts or a half-open original-time prefix when `seconds` is provided.
/// Joins outcomes, including those outside the prefix.
/// Scans twice so memory usage follows selected starts rather than the full event history.
/// Whole-object requests are decoded but cannot be replayed without a known object length.
pub fn load_requests(path: &Path, seconds: Option<u64>) -> Result<TraceRequests> {
    let duration = seconds
        .map(|seconds| {
            seconds
                .checked_mul(1_000_000_000)
                .filter(|&ns| ns > 0)
                .ok_or("trace seconds must be positive and fit in nanoseconds")
        })
        .transpose()?;
    let mut first = u64::MAX;
    let mut requests = Vec::new();
    visit_events(path, |event| {
        if event.kind == 0 {
            first = first.min(event.time_ns);
            if duration.is_none_or(|duration| event.time_ns - first < duration) {
                requests.push(Request {
                    request_id: event.request_id,
                    time_ns: event.time_ns,
                    object_key: event.object_key,
                    range: event.requested,
                });
            }
        }
        Ok(())
    })?;
    requests.retain(|request| duration.is_none_or(|duration| request.time_ns - first < duration));
    if requests.is_empty() {
        return Err("selected trace has no request starts".into());
    }
    let mut indices = FxHashMap::default();
    for (index, request) in requests.iter().enumerate() {
        if indices.insert(request.request_id, index).is_some() {
            return Err("duplicate request start ID".into());
        }
        if request.range == ByteRange::new(0, u64::MAX).unwrap() {
            return Err("whole-object replay requires a known object length".into());
        }
    }
    let mut completed = vec![false; requests.len()];
    let mut outcomes = [0; 6];
    visit_events(path, |event| {
        if event.kind == 0 {
            return Ok(());
        }
        if let Some(&index) = indices.get(&event.request_id) {
            let request = &requests[index];
            if completed[index] {
                return Err("duplicate request outcome ID".into());
            }
            if event.object_key != request.object_key
                || event.requested != request.range
                || event.time_ns < request.time_ns
            {
                return Err("request outcome disagrees with its start".into());
            }
            completed[index] = true;
            outcomes[event.kind as usize] += 1;
        }
        Ok(())
    })?;
    let incomplete = completed.iter().filter(|&&value| !value).count();
    for request in &mut requests {
        request.time_ns -= first;
    }
    requests.sort_unstable_by_key(|request| (request.time_ns, request.request_id));
    Ok(TraceRequests {
        requests,
        outcomes,
        incomplete,
    })
}

fn visit_events(path: &Path, mut visit: impl FnMut(Event) -> Result<()>) -> Result<()> {
    let mut read_package = |reader: &mut dyn Read| -> Result<()> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes)?;
        for event in decode_package(&bytes)? {
            visit(event?)?;
        }
        Ok(())
    };
    let mut file = BufReader::new(File::open(path)?);
    if path.extension().is_some_and(|extension| extension == "feuer") {
        return read_package(&mut file);
    }
    if !path.as_os_str().to_string_lossy().ends_with(".tar.zst") {
        return Err("input must be a .feuer package or a single-session .tar.zst archive".into());
    }
    let mut archive = tar::Archive::new(zstd::Decoder::new(file)?);
    let mut session = None;
    for entry in archive.entries()? {
        let mut entry = entry?;
        let name = entry.path()?.into_owned();
        if name.extension().is_some_and(|extension| extension == "feuer") {
            if !entry.header().entry_type().is_file() {
                return Err("trace package must be a regular archive member".into());
            }
            let parent = name.parent().unwrap().to_owned();
            if session.as_ref().is_some_and(|session| session != &parent) {
                return Err("archive contains multiple trace sessions with independent request IDs".into());
            }
            session = Some(parent);
            read_package(&mut entry)?;
        }
    }
    if session.is_none() {
        return Err("archive contains no .feuer packages".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(events: &[Event]) -> Vec<u8> {
        let mut bytes = b"FETR\x01\0\0\0".to_vec();
        for event in events {
            bytes.push(event.kind);
            bytes.extend(event.request_id.to_le_bytes());
            bytes.extend(event.time_ns.to_le_bytes());
            bytes.extend(event.object_key.0.to_le_bytes());
            bytes.extend(event.requested.start().to_le_bytes());
            bytes.extend(event.requested.end().to_le_bytes());
            let downloaded = event.downloaded.unwrap_or(ByteRange::new(0, 0).unwrap());
            bytes.extend(downloaded.start().to_le_bytes());
            bytes.extend(downloaded.end().to_le_bytes());
        }
        bytes
    }

    fn event(kind: u8, request_id: u64, time_ns: u64) -> Event {
        Event {
            kind,
            request_id,
            time_ns,
            object_key: ObjectKeyHash(0x112233445566778899aabbccddeeff00),
            requested: ByteRange::new(10, 20).unwrap(),
            downloaded: match kind {
                3 => Some(ByteRange::new(5, 25).unwrap()),
                5 => Some(ByteRange::new(0, 5).unwrap()),
                _ => None,
            },
        }
    }

    fn archive(path: &Path, packages: &[(&str, Vec<u8>)]) {
        let encoder = zstd::Encoder::new(File::create(path).unwrap(), 1).unwrap();
        let mut archive = tar::Builder::new(encoder);
        for (name, bytes) in packages {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o600);
            header.set_cksum();
            archive.append_data(&mut header, name, bytes.as_slice()).unwrap();
        }
        archive.into_inner().unwrap().finish().unwrap();
    }

    #[test]
    fn decodes_all_kinds_hashes_and_ranges() {
        let events: Vec<_> = (0..=5).map(|kind| event(kind, 42, 100)).collect();
        let bytes = package(&events);
        let decoded: Vec<_> = decode_package(&bytes).unwrap().collect::<Result<_>>().unwrap();
        assert_eq!(decoded, events);
        let mut whole = event(3, 1, 0);
        whole.requested = ByteRange::new(0, u64::MAX).unwrap();
        whole.downloaded = Some(ByteRange::new(0, 0).unwrap());
        assert_eq!(
            decode_package(&package(&[whole])).unwrap().next().unwrap().unwrap(),
            whole
        );
    }

    #[test]
    fn rejects_malformed_packages_and_events() {
        let valid = package(&[event(0, 1, 0)]);
        assert!(decode_package(&[]).is_err());
        assert!(decode_package(&valid[..8]).is_err());
        assert!(decode_package(&valid[..valid.len() - 1]).is_err());
        for (offset, value) in [(0, b'Q'), (4, 2), (8, 6), (8 + 49, 1), (8 + 33, 21)] {
            let mut bad = valid.clone();
            bad[offset] = value;
            assert!(
                decode_package(&bad)
                    .and_then(|events| events.collect::<Result<Vec<_>>>())
                    .is_err()
            );
        }
        let mut bad = event(3, 1, 0);
        bad.downloaded = Some(ByteRange::new(0, 5).unwrap());
        assert!(decode_package(&package(&[bad])).unwrap().next().unwrap().is_err());
        bad.kind = 5;
        bad.downloaded = Some(bad.requested);
        assert!(decode_package(&package(&[bad])).unwrap().next().unwrap().is_err());
    }

    #[test]
    fn sorts_starts_and_joins_outcomes_across_packages_and_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trace.tar.zst");
        archive(
            &path,
            &[
                (
                    "run/1.feuer",
                    package(&[event(0, 2, 600), event(0, 3, 100), event(0, 1, 100)]),
                ),
                (
                    "run/0.feuer",
                    package(&[
                        event(3, 2, 2_000_000_000),
                        event(1, 1, 101),
                        event(0, 4, 1_000_000_100),
                        event(4, 4, 1_000_000_101),
                        event(0, 5, 181_000_000_100),
                        event(1, 5, 181_000_000_101),
                    ]),
                ),
            ],
        );
        let trace = load_requests(&path, Some(1)).unwrap();
        assert_eq!(
            trace
                .requests
                .iter()
                .map(|r| (r.request_id, r.time_ns))
                .collect::<Vec<_>>(),
            [(1, 0), (3, 0), (2, 500)]
        );
        assert_eq!(trace.requests[0].object_key, event(0, 1, 0).object_key);
        assert_eq!(trace.outcomes, [0, 1, 0, 1, 0, 0]);
        assert_eq!(trace.incomplete, 1);

        let trace = load_requests(&path, None).unwrap();
        assert_eq!(
            trace
                .requests
                .iter()
                .map(|r| (r.request_id, r.time_ns))
                .collect::<Vec<_>>(),
            [(1, 0), (3, 0), (2, 500), (4, 1_000_000_000), (5, 181_000_000_000)]
        );
        assert_eq!(trace.outcomes, [0, 2, 0, 1, 1, 0]);
        assert_eq!(trace.incomplete, 1);
    }

    #[test]
    fn rejects_mixed_sessions_duplicates_and_inconsistent_outcomes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trace.tar.zst");
        archive(
            &path,
            &[
                ("a/0.feuer", package(&[event(0, 1, 0)])),
                ("b/0.feuer", package(&[event(0, 2, 1)])),
            ],
        );
        assert!(load_requests(&path, Some(1)).is_err());
        let path = dir.path().join("trace.feuer");
        let mut mismatched = event(1, 1, 1);
        mismatched.object_key = ObjectKeyHash(7);
        for events in [
            vec![event(0, 1, 0), event(0, 1, 1)],
            vec![event(0, 1, 0), event(1, 1, 1), event(1, 1, 2)],
            vec![event(0, 1, 0), mismatched],
        ] {
            std::fs::write(&path, package(&events)).unwrap();
            assert!(load_requests(&path, Some(1)).is_err());
        }
        assert!(load_requests(&path, Some(0)).is_err());
        assert!(load_requests(&path, Some(u64::MAX)).is_err());
        let mut whole = event(0, 1, 0);
        whole.requested = ByteRange::new(0, u64::MAX).unwrap();
        std::fs::write(&path, package(&[whole])).unwrap();
        assert!(load_requests(&path, Some(1)).is_err());
    }

    #[test]
    fn preserves_empty_ranges_and_reports_failed_outcomes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.feuer");
        let mut start = event(0, 1, 0);
        start.requested = ByteRange::new(10, 10).unwrap();
        let outcome = Event {
            kind: 4,
            time_ns: 1,
            ..start
        };
        std::fs::write(&path, package(&[start, outcome])).unwrap();
        let trace = load_requests(&path, None).unwrap();
        assert!(trace.requests[0].range.is_empty());
        assert_eq!(trace.outcomes[4], 1);
        assert_eq!(trace.incomplete, 0);
    }
}
