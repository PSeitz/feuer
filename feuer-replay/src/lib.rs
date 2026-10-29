//! FETR reading and chronological request selection for cache replays.

use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
};

use feuer_types::ObjectKeyHash;
use rustc_hash::FxHashMap;

/// Errors from trace decoding, input files, or replay preparation.
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// A recorded request start or lookup outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    /// 0 start, 1 memory hit, 2 disk hit, 3 callback success, 4 callback error, 5 miss.
    pub kind: u8,
    /// Identity joining this request's start and outcome within one trace session.
    pub request_id: u64,
    /// Nanoseconds since access-trace creation. Package order can differ from timestamp order.
    pub time_ns: u64,
    /// The original object identity, already hashed by the writer.
    pub object_key: ObjectKeyHash,
    /// Object length from a successful lookup.
    pub payload_len: Option<u64>,
}

/// Validates a package header and length, then decodes and validates each event.
/// Consuming the iterator validates every event. Missing events or packages can still go undetected.
pub fn decode_package(package: &[u8]) -> Result<impl Iterator<Item = Result<Event>> + '_> {
    if package.get(..8) != Some(b"FETR\x01\0\0\0") {
        return Err("expected FETR v1 package header".into());
    }
    let event_size = 41;
    if package.len() == 8 || !(package.len() - 8).is_multiple_of(event_size) {
        return Err("expected nonempty fixed-size FETR events".into());
    }
    Ok(package[8..].chunks_exact(event_size).map(move |event_bytes| {
        let u64_at = |offset| u64::from_le_bytes(event_bytes[offset..offset + 8].try_into().unwrap());
        let kind = event_bytes[0];
        if kind > 5 {
            return Err("unknown FETR v1 event kind".into());
        }
        if !(1..=3).contains(&kind) && u64_at(33) != 0 {
            return Err("unexpected payload length for FETR event kind".into());
        }
        Ok(Event {
            kind,
            request_id: u64_at(1),
            time_ns: u64_at(9),
            object_key: ObjectKeyHash(u128::from_le_bytes(event_bytes[17..33].try_into().unwrap())),
            payload_len: (1..=3).contains(&kind).then(|| u64_at(33)),
        })
    }))
}

/// An object request to replay.
#[derive(Clone, Copy, Debug)]
pub struct Request {
    /// Identity used to break ties between equal arrival timestamps.
    pub request_id: u64,
    /// Nanoseconds relative to the earliest recorded request start.
    pub time_ns: u64,
    /// Recorded object hash.
    pub object_key: ObjectKeyHash,
    /// Object payload length in bytes.
    pub payload_len: u64,
}

/// Selected request starts, with recorded outcomes reported separately from synthetic replay results.
pub struct TraceRequests {
    /// Requests in timestamp order, with request ID as the tie breaker.
    pub requests: Vec<Request>,
    /// Counts of recorded outcomes for selected starts, indexed by FETR kind.
    pub outcomes: [usize; 6],
    /// Selected starts whose outcomes are unknown.
    pub incomplete: usize,
}

/// Reads a single `.feuer` package or a `.tar.zst` archive containing one trace session.
/// Selects all starts or a half-open original-time prefix when `seconds` is provided.
/// Joins outcomes, including those outside the prefix.
/// Scans twice so memory usage follows selected starts rather than the full event history.
/// Object lengths come from successful outcomes, including outcomes outside the selected prefix.
pub fn load_requests(path: &Path, seconds: Option<u64>) -> Result<TraceRequests> {
    let prefix_ns = seconds
        .map(|seconds| {
            seconds
                .checked_mul(1_000_000_000)
                .filter(|&ns| ns > 0)
                .ok_or("trace seconds must be positive and fit in nanoseconds")
        })
        .transpose()?;
    let mut first_start_ns = u64::MAX;
    let mut requests = Vec::new();
    visit_events(path, |event| {
        if event.kind == 0 {
            first_start_ns = first_start_ns.min(event.time_ns);
            if prefix_ns.is_none_or(|prefix_ns| event.time_ns - first_start_ns < prefix_ns) {
                requests.push(event);
            }
        }
        Ok(())
    })?;
    requests.retain(|request| prefix_ns.is_none_or(|prefix_ns| request.time_ns - first_start_ns < prefix_ns));
    if requests.is_empty() {
        return Err("selected trace has no request starts".into());
    }
    let mut starts_by_id = FxHashMap::default();
    let mut object_lengths = FxHashMap::default();
    for request in requests {
        object_lengths.insert(request.object_key, None);
        if starts_by_id.insert(request.request_id, (request, false)).is_some() {
            return Err("duplicate request start ID".into());
        }
    }
    let mut outcomes = [0; 6];
    visit_events(path, |event| {
        if event.kind == 0 {
            return Ok(());
        }
        if let Some(length) = object_lengths.get_mut(&event.object_key)
            && let Some(observed) = event.payload_len
        {
            if length.is_some_and(|previous| previous != observed) {
                return Err("immutable object has inconsistent payload lengths".into());
            }
            *length = Some(observed);
        }
        if let Some((request, has_outcome)) = starts_by_id.get_mut(&event.request_id) {
            if *has_outcome {
                return Err("duplicate request outcome ID".into());
            }
            if event.object_key != request.object_key || event.time_ns < request.time_ns {
                return Err("request outcome disagrees with its start".into());
            }
            *has_outcome = true;
            outcomes[event.kind as usize] += 1;
        }
        Ok(())
    })?;
    let incomplete = starts_by_id.len() - outcomes.iter().sum::<usize>();
    let mut requests: Vec<_> = starts_by_id
        .into_values()
        .map(|(event, _)| {
            Ok(Request {
                request_id: event.request_id,
                time_ns: event.time_ns - first_start_ns,
                object_key: event.object_key,
                payload_len: object_lengths[&event.object_key]
                    .ok_or("object replay requires a successful outcome with its payload length")?,
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
        let mut package = Vec::new();
        reader.read_to_end(&mut package)?;
        decode_package(&package)?.try_for_each(|event| visit(event?))
    };
    let mut file = BufReader::new(File::open(path)?);
    if path.extension().is_some_and(|extension| extension == "feuer") {
        return read_package(&mut file);
    }
    if !path.as_os_str().to_string_lossy().ends_with(".tar.zst") {
        return Err("input must be a .feuer package or a single-session .tar.zst archive".into());
    }
    let mut archive = tar::Archive::new(zstd::Decoder::new(file)?);
    let mut session_directory = None;
    for entry in archive.entries()? {
        let mut entry = entry?;
        let name = entry.path()?.into_owned();
        if name.extension().is_some_and(|extension| extension == "feuer") {
            if !entry.header().entry_type().is_file() {
                return Err("trace package must be a regular archive member".into());
            }
            let parent = name.parent().unwrap().to_owned();
            if session_directory.as_ref().is_some_and(|directory| directory != &parent) {
                return Err("archive contains multiple trace sessions with independent request IDs".into());
            }
            session_directory = Some(parent);
            read_package(&mut entry)?;
        }
    }
    if session_directory.is_none() {
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
            bytes.extend(event.payload_len.unwrap_or(0).to_le_bytes());
        }
        bytes
    }

    fn event(kind: u8, request_id: u64, time_ns: u64) -> Event {
        Event {
            kind,
            request_id,
            time_ns,
            object_key: ObjectKeyHash(0x112233445566778899aabbccddeeff00),
            payload_len: (1..=3).contains(&kind).then_some(10),
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
    fn decodes_all_kinds_hashes_and_lengths() {
        let events: Vec<_> = (0..=5).map(|kind| event(kind, 42, 100)).collect();
        let bytes = package(&events);
        let decoded: Vec<_> = decode_package(&bytes).unwrap().collect::<Result<_>>().unwrap();
        assert_eq!(decoded, events);
    }

    #[test]
    fn decodes_objects_and_rejects_invalid_events() {
        let mut bytes = b"FETR\x01\0\0\0".to_vec();
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
            [0, 1, 2, 3, 4, 5]
        );
        assert_eq!(events[3].payload_len, Some(4));
        assert!(events.iter().all(|event| event.object_key == ObjectKeyHash(123)));
        assert!(decode_package(&bytes[..bytes.len() - 1]).is_err());
        for (offset, value) in [(8, 6), (8 + 33, 1)] {
            let mut bad = bytes.clone();
            bad[offset] = value;
            assert!(decode_package(&bad).unwrap().next().unwrap().is_err());
        }
    }

    fn object_package(events: &[(u8, u64, u64, u64)]) -> Vec<u8> {
        let mut bytes = b"FETR\x01\0\0\0".to_vec();
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
                let access_trace = load_requests(&path, Some(1)).unwrap();
                assert_eq!(access_trace.requests.len(), 1);
                assert_eq!(access_trace.requests[0].object_key, ObjectKeyHash(123));
                assert_eq!(access_trace.requests[0].payload_len, payload_len);
                assert_eq!(access_trace.incomplete, 1);
                assert_eq!(access_trace.outcomes, [0; 6]);
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
        for (offset, value) in [(0, b'Q'), (4, 0), (4, 255), (8, 6), (8 + 33, 1)] {
            let mut bad = valid.clone();
            bad[offset] = value;
            assert!(
                decode_package(&bad)
                    .and_then(|events| events.collect::<Result<Vec<_>>>())
                    .is_err()
            );
        }
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
        let access_trace = load_requests(&path, Some(1)).unwrap();
        assert_eq!(
            access_trace
                .requests
                .iter()
                .map(|r| (r.request_id, r.time_ns))
                .collect::<Vec<_>>(),
            [(1, 0), (3, 0), (2, 500)]
        );
        assert_eq!(access_trace.requests[0].object_key, event(0, 1, 0).object_key);
        assert_eq!(access_trace.requests[0].payload_len, 10);
        assert_eq!(access_trace.outcomes, [0, 1, 0, 1, 0, 0]);
        assert_eq!(access_trace.incomplete, 1);

        let access_trace = load_requests(&path, None).unwrap();
        assert_eq!(
            access_trace
                .requests
                .iter()
                .map(|r| (r.request_id, r.time_ns))
                .collect::<Vec<_>>(),
            [(1, 0), (3, 0), (2, 500), (4, 1_000_000_000), (5, 181_000_000_000)]
        );
        assert_eq!(access_trace.outcomes, [0, 2, 0, 1, 1, 0]);
        assert_eq!(access_trace.incomplete, 1);
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
        std::fs::write(&path, package(&[event(0, 1, 0)])).unwrap();
        assert!(load_requests(&path, Some(1)).is_err());
    }

    #[test]
    fn preserves_empty_objects_and_reports_errors_and_misses() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.feuer");
        for kind in [4, 5] {
            std::fs::write(
                &path,
                object_package(&[(0, 1, 0, 0), (kind, 1, 1, 0), (0, 2, 2, 0), (3, 2, 3, 0)]),
            )
            .unwrap();
            let access_trace = load_requests(&path, None).unwrap();
            assert!(access_trace.requests.iter().all(|request| request.payload_len == 0));
            assert_eq!(access_trace.outcomes[kind as usize], 1);
            assert_eq!(access_trace.outcomes[3], 1);
            assert_eq!(access_trace.incomplete, 0);
        }
    }
}
