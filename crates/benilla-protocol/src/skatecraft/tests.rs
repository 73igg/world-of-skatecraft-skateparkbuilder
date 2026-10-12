use super::*;

fn snapshot() -> Snapshot {
    let root = [1., 0., 0., 0., 1., 0., 0., 0., 1., -12000., 200., 4000.];
    let bones = (0..36)
        .map(|i| {
            let mut bone = [1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0.];
            bone[9] = i as f32 * 0.0125;
            bone[10] = 0.6;
            bone
        })
        .collect();
    Snapshot {
        generation: 42,
        sequence: 500,
        source_ms: 2_000_000,
        flags: 0,
        root,
        velocity: [12.5, -3.25, 4.],
        facing: 2.5,
        state: "PhysicsAir".into(),
        bones,
    }
}

#[test]
fn golden_control_packets_are_stable() {
    assert_eq!(encode(&Message::Hello).unwrap(), [1, 0, 0]);
    assert_eq!(encode(&Message::Welcome).unwrap(), [1, 0, 1]);
    assert_eq!(
        encode(&Message::Stop {
            generation: 0x0807_0605_0403_0201
        })
        .unwrap(),
        [1, 0, 4, 1, 2, 3, 4, 5, 6, 7, 8]
    );
}

#[test]
fn full_pose_round_trip_preserves_every_float_exactly() {
    let message = Message::Snapshot(snapshot());
    let bytes = encode(&message).unwrap();
    assert_eq!(decode(&bytes).unwrap(), message);
    assert!(
        bytes.len() < 36 * 48,
        "independent full pose should compress"
    );
}

#[test]
fn maximum_pose_round_trips_within_the_packet_budget() {
    let mut pose = snapshot();
    pose.bones.resize(MAX_BONES, pose.bones[0]);
    let bytes = encode(&Message::Snapshot(pose.clone())).unwrap();
    assert!(bytes.len() < MAX_BODY_BYTES);
    assert_eq!(decode(&bytes).unwrap(), Message::Snapshot(pose));
}

#[test]
fn dictionary_and_presentation_events_round_trip() {
    for message in [
        Message::Definition(Definition {
            generation: 1,
            fingerprint: [3; 20],
            names: vec!["HIPS".into(), "SKATEBOARD_ROOT".into()],
        }),
        Message::Events {
            generation: 1,
            events: vec![Event {
                id: 5,
                source_ms: 700,
                kind: EventKind::Land,
                strength: 2.75,
            }],
        },
    ] {
        assert_eq!(decode(&encode(&message).unwrap()).unwrap(), message);
    }
}

#[test]
fn server_routing_context_is_outside_the_owner_payload() {
    let relay = Relay {
        guid: 0x0102_0304_0506_0708,
        server_ms: 100,
        map: 0,
        instance: 2,
        message: Message::Snapshot(snapshot()),
    };
    let bytes = encode_relay(&relay).unwrap();
    assert_eq!(&bytes[..8], &[8, 7, 6, 5, 4, 3, 2, 1]);
    assert_eq!(decode_relay(&bytes).unwrap(), relay);
}

#[test]
fn truncated_packets_fail_at_every_boundary() {
    for message in [
        Message::Snapshot(snapshot()),
        Message::Definition(Definition {
            generation: 1,
            fingerprint: [0; 20],
            names: vec!["HIPS".into()],
        }),
    ] {
        let bytes = encode(&message).unwrap();
        for end in 0..bytes.len() {
            assert!(
                decode(&bytes[..end]).is_err(),
                "accepted truncated packet at {end}/{}",
                bytes.len()
            );
        }
    }
}

#[test]
fn versions_unknown_kinds_and_trailing_data_are_rejected() {
    assert!(decode(&[2, 0, 0]).is_err());
    assert!(decode(&[1, 0, 255]).is_err());
    assert!(decode(&[1, 0, 0, 0]).is_err());
    let mut bytes = encode(&Message::Snapshot(snapshot())).unwrap();
    bytes.push(0);
    assert!(decode(&bytes).is_err());
}

#[test]
fn inflated_pose_size_is_bounded_even_for_a_compression_bomb() {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(&vec![0; MAX_BODY_BYTES * 100]).unwrap();
    let mut bytes = vec![1, 0, 3];
    bytes.extend_from_slice(&(MAX_BODY_BYTES as u16).to_le_bytes());
    bytes.extend_from_slice(&encoder.finish().unwrap());
    assert!(decode(&bytes).is_err());
}

#[test]
fn empty_duplicate_and_oversized_layouts_are_rejected() {
    for names in [
        vec![],
        vec!["HIPS".into(), "HIPS".into()],
        vec!["x".repeat(65)],
        vec!["HIPS".into(); MAX_BONES + 1],
    ] {
        assert!(encode(&Message::Definition(Definition {
            generation: 1,
            fingerprint: [0; 20],
            names
        }))
        .is_err());
    }
    let mut bytes = vec![1, 0, 2];
    bytes.extend_from_slice(&1u64.to_le_bytes());
    bytes.extend_from_slice(&[0; 20]);
    bytes.extend_from_slice(&((MAX_BONES + 1) as u16).to_le_bytes());
    assert!(decode(&bytes).is_err());
}

#[test]
fn nonfinite_and_out_of_range_data_never_reaches_presentation() {
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1_000_000.] {
        let mut pose = snapshot();
        pose.root[9] = value;
        assert!(encode(&Message::Snapshot(pose)).is_err());
    }
    let mut pose = snapshot();
    pose.flags = 128;
    assert!(encode(&Message::Snapshot(pose)).is_err());
    assert!(encode(&Message::Stop { generation: 0 }).is_err());
    let mut singular = snapshot();
    singular.root[..9].fill(0.);
    assert!(encode(&Message::Snapshot(singular)).is_err());
}

#[test]
fn handshakes_have_an_explicit_direction_and_identity() {
    assert!(decode_client(&[1, 0, 1]).is_err());
    assert_eq!(decode_client(&[1, 0, 0]).unwrap(), Message::Hello);
    for (guid, message) in [
        (0, Message::Hello),
        (1, Message::Welcome),
        (0, Message::Snapshot(snapshot())),
    ] {
        assert!(encode_relay(&Relay {
            guid,
            server_ms: 0,
            map: 0,
            instance: 0,
            message
        })
        .is_err());
    }
    let welcome = Relay {
        guid: 0,
        server_ms: 10,
        map: 0,
        instance: 0,
        message: Message::Welcome,
    };
    assert_eq!(
        decode_relay(&encode_relay(&welcome).unwrap()).unwrap(),
        welcome
    );
}
