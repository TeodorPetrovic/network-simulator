use super::*;
use std::{env, fs};
use tokio::{net::UdpSocket, time::timeout};

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires the upstream uBridge executable; set NETWORK_SIMULATOR_TEST_UBRIDGE"]
    async fn live_filter_suspend_and_capture_on_udp_link() {
        let binary = env::var_os("NETWORK_SIMULATOR_TEST_UBRIDGE")
            .expect("NETWORK_SIMULATOR_TEST_UBRIDGE names the upstream uBridge executable");
        let directory = tempfile::tempdir().unwrap();
        let runtime = UbridgeRuntime::with_paths(binary, directory.path().join("ubridge.sock"));
        let ids = [
            uuid::Uuid::new_v4().to_string(),
            uuid::Uuid::new_v4().to_string(),
        ];
        let link_ports = [
            runtime.reserve_udp_port().await.unwrap(),
            runtime.reserve_udp_port().await.unwrap(),
        ];
        let runtime_ports = [
            runtime.reserve_udp_port().await.unwrap(),
            runtime.reserve_udp_port().await.unwrap(),
        ];
        let peers = [
            UdpSocket::bind("127.0.0.1:0").await.unwrap(),
            UdpSocket::bind("127.0.0.1:0").await.unwrap(),
        ];
        let frame = [
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x02, 0x00, 0x00, 0x00, 0x00, 0x01, 0x08, 0x00,
            0x45, 0x00, 0x00, 0x2e, 0x12, 0x34, 0x00, 0x00, 0x40, 0x11, 0x00, 0x00, 10, 0, 0, 1,
            10, 0, 0, 2,
        ];
        let run: Result<(), RuntimeError> = async {
            for index in 0..2 {
                runtime
                    .attach_link_endpoint(
                        &ids[index],
                        link_ports[index],
                        link_ports[1 - index],
                        Some(runtime_ports[index]),
                        Some(peers[index].local_addr().unwrap().port()),
                        None,
                        &serde_json::json!({}),
                        false,
                        None,
                    )
                    .await?;
            }
            let mut received = [0_u8; 2048];
            peers[0]
                .send_to(&frame, ("127.0.0.1", runtime_ports[0]))
                .await
                .map_err(|error| RuntimeError::Failed(error.to_string()))?;
            let size = timeout(Duration::from_secs(2), peers[1].recv(&mut received))
                .await
                .map_err(|_| RuntimeError::Failed("baseline frame timed out".into()))?
                .map_err(|error| RuntimeError::Failed(error.to_string()))?;
            if received[..size] != frame {
                return Err(RuntimeError::Failed("baseline frame changed".into()));
            }

            runtime
                .update_link_endpoint(&ids[0], &serde_json::json!({"frequency_drop":[-1]}), false)
                .await?;
            peers[0]
                .send_to(&frame, ("127.0.0.1", runtime_ports[0]))
                .await
                .map_err(|error| RuntimeError::Failed(error.to_string()))?;
            if timeout(Duration::from_millis(400), peers[1].recv(&mut received))
                .await
                .is_ok()
            {
                return Err(RuntimeError::Failed(
                    "frequency_drop did not block the frame".into(),
                ));
            }
            runtime
                .update_link_endpoint(&ids[0], &serde_json::json!({}), true)
                .await?;
            peers[0]
                .send_to(&frame, ("127.0.0.1", runtime_ports[0]))
                .await
                .map_err(|error| RuntimeError::Failed(error.to_string()))?;
            if timeout(Duration::from_millis(400), peers[1].recv(&mut received))
                .await
                .is_ok()
            {
                return Err(RuntimeError::Failed(
                    "suspended link forwarded a frame".into(),
                ));
            }
            runtime
                .update_link_endpoint(&ids[0], &serde_json::json!({}), false)
                .await?;
            let capture = directory.path().join("live-link.pcap");
            runtime
                .start_link_capture(&ids[0], capture.to_str().unwrap())
                .await?;
            peers[0]
                .send_to(&frame, ("127.0.0.1", runtime_ports[0]))
                .await
                .map_err(|error| RuntimeError::Failed(error.to_string()))?;
            let size = timeout(Duration::from_secs(2), peers[1].recv(&mut received))
                .await
                .map_err(|_| RuntimeError::Failed("resumed frame timed out".into()))?
                .map_err(|error| RuntimeError::Failed(error.to_string()))?;
            if received[..size] != frame {
                return Err(RuntimeError::Failed("resumed frame changed".into()));
            }
            runtime.stop_link_capture(&ids[0]).await?;
            let bytes =
                fs::read(&capture).map_err(|error| RuntimeError::Failed(error.to_string()))?;
            if bytes.len() < 24 + 16 + frame.len() {
                return Err(RuntimeError::Failed(format!(
                    "capture is too short: {} bytes",
                    bytes.len()
                )));
            }
            Ok(())
        }
        .await;
        for id in &ids {
            let _ = runtime.delete_link_endpoint(id).await;
        }
        runtime.shutdown().await;
        assert!(run.is_ok(), "live relay test failed: {run:?}");
    }
}
