use super::*;

use super::*;

#[test]
fn request_span_uses_only_the_path_and_drops_query_credentials() {
    let request = axum::http::Request::builder()
        .uri("/v3/projects/p1/notifications/ws?ticket=secret-ticket&after=12")
        .body(())
        .unwrap();
    assert_eq!(
        request_trace_path(&request),
        "/v3/projects/p1/notifications/ws"
    );
    let span = request_span(&request);
    assert!(span.metadata().unwrap().fields().field("path").is_some());
    assert!(!request_trace_path(&request).contains("secret-ticket"));
}
