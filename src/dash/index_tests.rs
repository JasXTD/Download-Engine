use super::*;

fn index(version: u8, time: u64, offset: u64, sizes: &[u32]) -> Vec<u8> {
    let mut body = vec![version, 0, 0, 0];
    body.extend(1u32.to_be_bytes());
    body.extend(1000u32.to_be_bytes());
    if version == 0 {
        body.extend((time as u32).to_be_bytes());
        body.extend((offset as u32).to_be_bytes());
    } else {
        body.extend(time.to_be_bytes());
        body.extend(offset.to_be_bytes());
    }
    body.extend(0u16.to_be_bytes());
    body.extend((sizes.len() as u16).to_be_bytes());
    for size in sizes {
        body.extend(size.to_be_bytes());
        body.extend(2000u32.to_be_bytes());
        body.extend(0x90000000u32.to_be_bytes());
    }
    let mut result = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    result.extend(b"sidx");
    result.extend(body);
    result
}
fn ranges(bytes: &[u8]) -> Result<Vec<Range>> {
    parse(
        bytes,
        &Range {
            start: 100,
            length: bytes.len() as u64,
        },
        10000,
        0,
        6000,
        true,
    )
}
#[test]
fn versions_and_extended_sizes_use_absolute_sidx_end_plus_first_offset() {
    for version in [0, 1] {
        let bytes = index(version, 0, 20, &[100, 200, 300]);
        let parts = ranges(&bytes).unwrap();
        assert_eq!(
            parts[0],
            Range {
                start: 100 + bytes.len() as u64 + 20,
                length: 100
            }
        );
        assert_eq!(
            parts[2],
            Range {
                start: 100 + bytes.len() as u64 + 320,
                length: 300
            }
        );
        let mut extended = 1u32.to_be_bytes().to_vec();
        extended.extend(b"sidx");
        extended.extend((bytes.len() as u64 + 8).to_be_bytes());
        extended.extend(&bytes[8..]);
        assert_eq!(
            ranges(&extended).unwrap()[0].start,
            100 + extended.len() as u64 + 20
        );
    }
}
#[test]
fn non_exact_ranges_skip_prefix_boxes_and_preserve_64_bit_times_and_offsets() {
    let mut bytes = 8u32.to_be_bytes().to_vec();
    bytes.extend(b"free");
    bytes.extend(index(1, 5_000_000_000, 6_000_000_000, &[200, 300, 400]));
    let start = 7_000_000_000;
    let result = parse(
        &bytes,
        &Range {
            start,
            length: bytes.len() as u64,
        },
        20_000_000_000,
        5_000_000_000_000,
        6000,
        false,
    )
    .unwrap();
    assert_eq!(result[0].start, start + bytes.len() as u64 + 6_000_000_000);
    assert!(
        parse(
            &bytes,
            &Range {
                start,
                length: bytes.len() as u64
            },
            20_000_000_000,
            5_000_000_000_000,
            6000,
            true
        )
        .is_err()
    );
}
#[test]
fn period_filtering_keeps_offsets_and_rejects_partial_leading_segments() {
    let bytes = index(0, 0, 0, &[100, 200, 300]);
    let request = Range {
        start: 100,
        length: bytes.len() as u64,
    };
    let selected = parse(&bytes, &request, 10000, 2_000_000, 2000, true).unwrap();
    assert_eq!(
        selected,
        vec![Range {
            start: 100 + bytes.len() as u64 + 100,
            length: 200
        }]
    );
    assert!(parse(&bytes, &request, 10000, 1_000_000, 2000, true).is_err());
    assert!(parse(&bytes, &request, 10000, 9_000_000, 2000, true).is_err());
}
#[test]
fn malformed_truncated_recursive_and_overflowing_indexes_fail_closed() {
    let bytes = index(0, 0, 0, &[100, 200, 300]);
    for length in 0..bytes.len() {
        assert!(
            ranges(&bytes[..length]).is_err(),
            "accepted truncated index at {length}"
        );
    }
    for bad in [
        index(2, 0, 0, &[100]),
        index(0, 0, 0, &[]),
        index(0, 0, 0, &[0]),
        index(0, 0, 0, &[0x80000001]),
        index(0, 0, 0, &[MAX_PART_BYTES as u32 + 1]),
        index(1, 0, u64::MAX, &[100]),
        index(1, u64::MAX, 0, &[100]),
    ] {
        assert!(ranges(&bad).is_err());
    }
    for (offset, value) in [(16, 0u32), (20, u32::MAX), (36, 0)] {
        let mut bad = bytes.clone();
        bad[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
        assert!(ranges(&bad).is_err());
    }
    let request = Range {
        start: 100,
        length: bytes.len() as u64,
    };
    assert!(
        parse(
            &bytes,
            &request,
            100 + bytes.len() as u64 + 599,
            0,
            6000,
            true
        )
        .is_err()
    );
    assert!(
        parse(
            &[bytes.clone(), bytes].concat(),
            &Range {
                start: 100,
                length: 136
            },
            10000,
            0,
            6000,
            false
        )
        .is_err()
    );
}
