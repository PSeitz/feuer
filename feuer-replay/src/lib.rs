//! FETR reading and chronological request selection for cache replays.

use std::{
    fs::File,
    io::{BufReader, Read},
    ops::Range,
    path::Path,
};

use feuer_types::ObjectKeyHash;
use rustc_hash::FxHashMap;

/// Errors from trace decoding, input files, or replay preparation.
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// One recorded request start or lookup outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    /// 0 start, 1 memory hit, 2 disk hit, 3 callback success, 4 callback error, 5 historical noncovering download, 6 miss.
    /// V2 miss kind 5 is normalized to 6.
    pub kind: u8,
    /// Identity joining this request's start and outcome within one trace session.
    pub request_id: u64,
    /// Nanoseconds since trace creation, not necessarily ordered in the package.
    pub time_ns: u64,
    /// The original object identity, already hashed by the writer.
    pub object_key: ObjectKeyHash,
    /// Historical v1 requested range. `0..u64::MAX` denotes an unknown-length object.
    pub requested: Option<Range<u64>>,
    /// Historical v1 callback range, present only for kinds 3 and 5.
    pub downloaded: Option<Range<u64>>,
    /// Complete object length from a successful v2 lookup.
    pub payload_len: Option<u64>,
}

/// Validates a package header and length, then decodes and validates each event.
/// Consuming the iterator validates the entire package, not trace completeness.
pub fn decode_package(data: &[u8]) -> Result<impl Iterator<Item = Result<Event>> + '_> {
    let (version, event_bytes) = match data.get(..8) {
        Some(b"FETR\x01\0\0\0") => (1, 65),
        Some(b"FETR\x02\0\0\0") => (2, 41),
        _ => return Err("expected FETR v1 or v2 package header".into()),
    };
    if data.len() == 8 || !(data.len() - 8).is_multiple_of(event_bytes) {
        return Err("expected nonempty fixed-size FETR events".into());
    }
    Ok(data[8..].chunks_exact(event_bytes).map(move |record| {
        let u64_at = |offset| u64::from_le_bytes(record[offset..offset + 8].try_into().unwrap());
        let kind = record[0];
        if version == 2 {
            if kind > 5 {
                return Err("unknown FETR v2 event kind".into());
            }
            if !(1..=3).contains(&kind) && u64_at(33) != 0 {
                return Err("unexpected payload length for FETR event kind".into());
            }
            return Ok(Event {
                kind: if kind == 5 { 6 } else { kind },
                request_id: u64_at(1),
                time_ns: u64_at(9),
                object_key: ObjectKeyHash(u128::from_le_bytes(record[17..33].try_into().unwrap())),
                requested: None,
                downloaded: None,
                payload_len: (1..=3).contains(&kind).then(|| u64_at(33)),
            });
        }
        if kind > 6 {
            return Err("unknown FETR event kind".into());
        }
        let range_at = |offset| -> Result<Range<u64>> {
            let range = u64_at(offset)..u64_at(offset + 8);
            if range.start > range.end {
                return Err("invalid FETR byte range".into());
            }
            Ok(range)
        };
        let requested = range_at(33)?;
        let downloaded = match kind {
            3 | 5 => Some(range_at(49)?),
            _ if u64_at(49) != 0 || u64_at(57) != 0 => {
                return Err("unexpected downloaded range for FETR event kind".into());
            }
            _ => None,
        };
        if let Some(downloaded) = &downloaded {
            let covers = downloaded.start <= requested.start && downloaded.end >= requested.end;
            if kind == 3 {
                if requested == (0..u64::MAX) {
                    if downloaded.start != 0 {
                        return Err("whole-object download must start at zero".into());
                    }
                } else if !covers {
                    return Err("callback success does not cover requested range".into());
                }
            } else if covers {
                return Err("noncovering download covers requested range".into());
            }
        }
        Ok(Event {
            kind,
            request_id: u64_at(1),
            time_ns: u64_at(9),
            object_key: ObjectKeyHash(u128::from_le_bytes(record[17..33].try_into().unwrap())),
            requested: Some(requested),
            downloaded,
            payload_len: None,
        })
    }))
}

/// One arrival to replay as a complete object.
#[derive(Clone, Copy, Debug)]
pub struct Request {
    /// Identity used to break ties between equal arrival timestamps.
    pub request_id: u64,
    /// Nanoseconds relative to the earliest recorded request start.
    pub time_ns: u64,
    /// Recorded object hash, or an identity derived from an exact historical range.
    pub object_key: ObjectKeyHash,
    /// Full object payload length in bytes.
    pub payload_len: u64,
}

/// Selected request starts, with recorded outcomes reported separately from synthetic replay results.
pub struct TraceRequests {
    /// Requests in timestamp order, with request ID as the tie breaker.
    pub requests: Vec<Request>,
    /// Counts of recorded outcomes for selected starts, indexed by FETR kind.
    pub outcomes: [usize; 7],
    /// Selected starts without a recorded outcome. These are not established cache misses.
    pub incomplete: usize,
}

/// Reads a single `.feuer` package or a `.tar.zst` archive containing one trace session.
/// Selects all starts or a half-open original-time prefix when `seconds` is provided.
/// Joins outcomes, including those outside the prefix.
/// Scans twice so memory usage follows selected starts rather than the full event history.
/// Object lengths come from successful outcomes, including outcomes outside the selected prefix.
/// Historical ranges become independent objects with lengths derived from their endpoints.
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
                requests.push(event);
            }
        }
        Ok(())
    })?;
    requests.retain(|request| duration.is_none_or(|duration| request.time_ns - first < duration));
    if requests.is_empty() {
        return Err("selected trace has no request starts".into());
    }
    let mut requests_by_id = FxHashMap::default();
    let mut object_lengths = FxHashMap::default();
    for request in &requests {
        if requests_by_id.insert(request.request_id, (request, false)).is_some() {
            return Err("duplicate request start ID".into());
        }
        if request.requested.as_ref().is_none_or(|range| *range == (0..u64::MAX)) {
            object_lengths.insert(request.object_key, None);
        }
    }
    let mut outcomes = [0; 7];
    visit_events(path, |event| {
        if event.kind == 0 {
            return Ok(());
        }
        if let Some(length) = object_lengths.get_mut(&event.object_key) {
            let observed = event.payload_len.or_else(|| {
                (event.kind == 3 && event.requested == Some(0..u64::MAX))
                    .then(|| event.downloaded.as_ref().map(|range| range.end - range.start))
                    .flatten()
            });
            if let Some(observed) = observed {
                if length.is_some_and(|previous| previous != observed) {
                    return Err("immutable object has inconsistent payload lengths".into());
                }
                *length = Some(observed);
            }
        }
        if let Some((request, completed)) = requests_by_id.get_mut(&event.request_id) {
            if *completed {
                return Err("duplicate request outcome ID".into());
            }
            if event.object_key != request.object_key
                || event.requested != request.requested
                || event.time_ns < request.time_ns
            {
                return Err("request outcome disagrees with its start".into());
            }
            *completed = true;
            outcomes[event.kind as usize] += 1;
        }
        Ok(())
    })?;
    let incomplete = requests_by_id.values().filter(|(_, completed)| !completed).count();
    let mut requests: Vec<_> = requests
        .into_iter()
        .map(|event| {
            let (object_key, payload_len) = match event.requested {
                Some(range) if range != (0..u64::MAX) => (
                    ObjectKeyHash::from(format!(
                        "slice-v1:{}:{}:{:032x}",
                        range.start, range.end, event.object_key.0,
                    )),
                    range.end - range.start,
                ),
                _ => (
                    event.object_key,
                    object_lengths[&event.object_key]
                        .ok_or("object replay requires a successful outcome with its payload length")?,
                ),
            };
            Ok(Request {
                request_id: event.request_id,
                time_ns: event.time_ns - first,
                object_key,
                payload_len,
            })
        })
        .collect::<Result<_>>()?;
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
        decode_package(&bytes)?.try_for_each(|event| visit(event?))
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
            let requested = event.requested.as_ref().unwrap();
            bytes.extend(requested.start.to_le_bytes());
            bytes.extend(requested.end.to_le_bytes());
            let downloaded = event.downloaded.clone().unwrap_or(0..0);
            bytes.extend(downloaded.start.to_le_bytes());
            bytes.extend(downloaded.end.to_le_bytes());
        }
        bytes
    }

    fn event(kind: u8, request_id: u64, time_ns: u64) -> Event {
        Event {
            kind,
            request_id,
            time_ns,
            object_key: ObjectKeyHash(0x112233445566778899aabbccddeeff00),
            requested: Some(10..20),
            downloaded: match kind {
                3 => Some(5..25),
                5 => Some(0..5),
                _ => None,
            },
            payload_len: None,
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
        let events: Vec<_> = (0..=6).map(|kind| event(kind, 42, 100)).collect();
        let bytes = package(&events);
        let decoded: Vec<_> = decode_package(&bytes).unwrap().collect::<Result<_>>().unwrap();
        assert_eq!(decoded, events);
        let mut whole = event(3, 1, 0);
        whole.requested = Some(0..u64::MAX);
        whole.downloaded = Some(0..0);
        assert_eq!(
            decode_package(&package(&[whole.clone()]))
                .unwrap()
                .next()
                .unwrap()
                .unwrap(),
            whole
        );
    }

    #[test]
    fn decodes_v2_objects_and_rejects_invalid_records() {
        let mut bytes = b"FETR\x02\0\0\0".to_vec();
        for kind in 0u8..=5 {
            bytes.push(kind);
            bytes.extend(42u64.to_le_bytes());
            bytes.extend(100u64.to_le_bytes());
            bytes.extend(123u128.to_le_bytes());
            bytes.extend(if kind == 3 { 4u64 } else { 0 }.to_le_bytes());
        }
        let events = decode_package(&bytes).unwrap().collect::<Result<Vec<_>>>().unwrap();
        assert_eq!(
            events.iter().map(|event| event.kind).collect::<Vec<_>>(),
            [0, 1, 2, 3, 4, 6]
        );
        assert_eq!(events[3].payload_len, Some(4));
        assert!(events.iter().all(|event| event.object_key == ObjectKeyHash(123)));
        assert!(
            events
                .iter()
                .all(|event| event.requested.is_none() && event.downloaded.is_none())
        );
        assert!(decode_package(&bytes[..bytes.len() - 1]).is_err());
        for (offset, value) in [(8, 6), (8 + 33, 1)] {
            let mut bad = bytes.clone();
            bad[offset] = value;
            assert!(decode_package(&bad).unwrap().next().unwrap().is_err());
        }
    }

    fn object_package(events: &[(u8, u64, u64, u64)]) -> Vec<u8> {
        let mut bytes = b"FETR\x02\0\0\0".to_vec();
        for &(kind, id, time_ns, payload_len) in events {
            bytes.push(kind);
            bytes.extend(id.to_le_bytes());
            bytes.extend(time_ns.to_le_bytes());
            bytes.extend(123u128.to_le_bytes());
            bytes.extend(payload_len.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn replays_objects_using_successful_lengths_beyond_the_selected_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("native.feuer");
        for kind in 1..=3 {
            for payload_len in [0, 17] {
                std::fs::write(
                    &path,
                    object_package(&[
                        (0, 1, 0, 0),
                        (0, 2, 2_000_000_000, 0),
                        (kind, 2, 3_000_000_000, payload_len),
                    ]),
                )
                .unwrap();
                let trace = load_requests(&path, Some(1)).unwrap();
                assert_eq!(trace.requests.len(), 1);
                assert_eq!(trace.requests[0].object_key, ObjectKeyHash(123));
                assert_eq!(trace.requests[0].payload_len, payload_len);
                assert_eq!(trace.incomplete, 1);
                assert_eq!(trace.outcomes, [0; 7]);
            }
        }
    }

    #[test]
    fn rejects_unknown_or_inconsistent_object_lengths() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("native.feuer");
        for (events, message) in [
            (vec![(0, 1, 0, 0), (4, 1, 1, 0)], "requires a successful outcome"),
            (
                vec![(0, 1, 0, 0), (3, 1, 1, 4), (0, 2, 2, 0), (1, 2, 3, 5)],
                "inconsistent payload lengths",
            ),
        ] {
            std::fs::write(&path, object_package(&events)).unwrap();
            assert!(load_requests(&path, None).err().unwrap().to_string().contains(message));
        }
    }

    #[test]
    fn rejects_malformed_packages_and_events() {
        let valid = package(&[event(0, 1, 0)]);
        assert!(decode_package(&[]).is_err());
        assert!(decode_package(&valid[..8]).is_err());
        assert!(decode_package(&valid[..valid.len() - 1]).is_err());
        for (offset, value) in [(0, b'Q'), (4, 3), (8, 7), (8 + 49, 1), (8 + 33, 21)] {
            let mut bad = valid.clone();
            bad[offset] = value;
            assert!(
                decode_package(&bad)
                    .and_then(|events| events.collect::<Result<Vec<_>>>())
                    .is_err()
            );
        }
        let mut bad = event(3, 1, 0);
        bad.downloaded = Some(0..5);
        assert!(
            decode_package(&package(&[bad.clone()]))
                .unwrap()
                .next()
                .unwrap()
                .is_err()
        );
        bad.kind = 5;
        bad.downloaded = bad.requested.clone();
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
        assert_eq!(
            trace.requests[0].object_key,
            ObjectKeyHash::from(format!("slice-v1:10:20:{:032x}", event(0, 1, 0).object_key.0))
        );
        assert_eq!(trace.requests[0].payload_len, 10);
        assert_eq!(trace.outcomes, [0, 1, 0, 1, 0, 0, 0]);
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
        assert_eq!(trace.outcomes, [0, 2, 0, 1, 1, 0, 0]);
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
        whole.requested = Some(0..u64::MAX);
        std::fs::write(&path, package(&[whole])).unwrap();
        assert!(load_requests(&path, Some(1)).is_err());
    }

    #[test]
    fn objects_preserve_lengths_but_separate_exact_ranges() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trace.feuer");
        let start = event(0, 1, 123);
        let mut events = vec![
            start.clone(),
            Event {
                request_id: 2,
                ..start.clone()
            },
        ];
        for (index, requested) in [11..21, 10..21, 10..10].into_iter().enumerate() {
            events.push(Event {
                request_id: index as u64 + 3,
                requested: Some(requested),
                ..start.clone()
            });
        }
        events.push(Event {
            request_id: 6,
            object_key: ObjectKeyHash(2),
            ..start
        });
        std::fs::write(&path, package(&events)).unwrap();
        let requests = load_requests(&path, None).unwrap().requests;
        assert_eq!(requests[0].object_key, requests[1].object_key);
        for request in &requests[2..] {
            assert_ne!(requests[0].object_key, request.object_key);
        }
        assert_eq!(
            requests.iter().map(|r| r.payload_len).collect::<Vec<_>>(),
            [10, 10, 10, 11, 0, 10]
        );
        assert!(requests.iter().all(|request| request.time_ns == 0));
    }

    #[test]
    fn preserves_empty_ranges_and_reports_errors_and_misses() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.feuer");
        let mut start = event(0, 1, 0);
        start.requested = Some(10..10);
        for kind in [4, 6] {
            let outcome = Event {
                kind,
                time_ns: 1,
                ..start.clone()
            };
            std::fs::write(&path, package(&[start.clone(), outcome])).unwrap();
            let trace = load_requests(&path, None).unwrap();
            assert_eq!(trace.requests[0].payload_len, 0);
            assert_eq!(trace.outcomes[kind as usize], 1);
            assert_eq!(trace.incomplete, 0);
        }
    }
}
