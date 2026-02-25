use std::time::Duration;

use tokio::time::timeout;
use zfs_core::{ProgramId, SectorFetchRequest, SectorFetchResponse, SectorRequest, SectorResponse};
use zfs_net::{NetworkConfig, NetworkEvent, NetworkService};

async fn wait_for_listen_addr(zode: &mut NetworkService) -> zfs_net::Multiaddr {
    timeout(Duration::from_secs(10), async {
        loop {
            if let Some(NetworkEvent::ListenAddress(addr)) = zode.next_event().await {
                return addr;
            }
        }
    })
    .await
    .expect("timed out waiting for listen address")
}

#[tokio::test]
async fn two_zode_connectivity() {
    let mut zode1 = NetworkService::new(NetworkConfig::new(
        "/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap(),
    ))
    .await
    .unwrap();

    let mut zode2 = NetworkService::new(NetworkConfig::new(
        "/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap(),
    ))
    .await
    .unwrap();

    let _zode1_addr = wait_for_listen_addr(&mut zode1).await;
    let zode2_addr = wait_for_listen_addr(&mut zode2).await;
    let zode2_zode_id = *zode2.local_zode_id();

    tokio::spawn(async move {
        loop {
            let _ = zode2.next_event().await;
        }
    });

    zode1.dial(zode2_addr).unwrap();

    let connected_peer = timeout(Duration::from_secs(10), async {
        loop {
            if let Some(NetworkEvent::PeerConnected(peer)) = zode1.next_event().await {
                return peer;
            }
        }
    })
    .await
    .expect("connection timed out");

    assert_eq!(connected_peer, zode2_zode_id);
}

#[tokio::test]
async fn sector_request_response_round_trip() {
    let mut zode1 = NetworkService::new(NetworkConfig::new(
        "/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap(),
    ))
    .await
    .unwrap();

    let mut zode2 = NetworkService::new(NetworkConfig::new(
        "/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap(),
    ))
    .await
    .unwrap();

    let _zode1_addr = wait_for_listen_addr(&mut zode1).await;
    let zode2_addr = wait_for_listen_addr(&mut zode2).await;
    let zode2_zode_id = *zode2.local_zode_id();

    tokio::spawn(async move {
        loop {
            if let Some(NetworkEvent::IncomingSectorRequest { channel, .. }) =
                zode2.next_event().await
            {
                let resp = SectorResponse::Fetch(SectorFetchResponse {
                    payload: Some(vec![0xAB; 64]),
                    error_code: None,
                });
                zode2
                    .send_sector_response(channel, resp)
                    .expect("send response");
            }
        }
    });

    zode1.dial(zode2_addr).unwrap();

    timeout(Duration::from_secs(10), async {
        loop {
            if let Some(NetworkEvent::PeerConnected(_)) = zode1.next_event().await {
                return;
            }
        }
    })
    .await
    .expect("connection timed out");

    let request = SectorRequest::Fetch(SectorFetchRequest {
        program_id: ProgramId::from([0u8; 32]),
        sector_id: zfs_core::SectorId::from_bytes(vec![1, 2, 3]),
    });
    zode1.send_sector_request(&zode2_zode_id, request);

    let response = timeout(Duration::from_secs(10), async {
        loop {
            if let Some(NetworkEvent::SectorRequestResult { response, .. }) =
                zode1.next_event().await
            {
                return *response;
            }
        }
    })
    .await
    .expect("sector response timed out");

    match response {
        SectorResponse::Fetch(f) => {
            assert_eq!(f.payload, Some(vec![0xAB; 64]));
            assert!(f.error_code.is_none());
        }
        other => panic!("expected Fetch response, got {other:?}"),
    }
}
