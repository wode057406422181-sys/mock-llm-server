use eventsource_stream::Eventsource;
use futures::StreamExt;
use mock_llm_server::{MockLlmServer, MockResponse};
use reqwest::Client;
use serde_json::json;

#[tokio::test]
async fn test_anthropic_text_response() {
    let script = vec![MockResponse::text("hello world")];
    let server = MockLlmServer::start(script).await.unwrap();

    let client = Client::builder().no_proxy().build().unwrap();
    let res = client
        .post(format!("{}/v1/messages", server.url()))
        .send()
        .await
        .unwrap();
    assert!(res.status().is_success());

    let mut stream = res.bytes_stream().eventsource();
    let mut events = Vec::new();
    while let Some(Ok(event)) = stream.next().await {
        events.push((event.event, event.data));
    }

    assert_eq!(events[0].0, "message_start");
    assert_eq!(events[1].0, "content_block_start");
    assert_eq!(events[2].0, "content_block_delta");
    assert!(events[2].1.contains("hello world"));
    assert_eq!(events[3].0, "content_block_stop");
    assert_eq!(events[4].0, "message_delta");
    assert_eq!(events[5].0, "message_stop");

    assert_eq!(server.remaining().await, 0);
}

#[tokio::test]
async fn test_openai_text_response() {
    let script = vec![MockResponse::text("hello world")];
    let server = MockLlmServer::start(script).await.unwrap();

    let client = Client::builder().no_proxy().build().unwrap();
    let res = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .send()
        .await
        .unwrap();
    assert!(res.status().is_success());

    let mut stream = res.bytes_stream().eventsource();
    let mut events = Vec::new();
    while let Some(Ok(event)) = stream.next().await {
        events.push(event.data);
    }

    assert!(events.iter().any(|d| d.contains("hello world")));
    assert_eq!(events.last().unwrap(), "[DONE]");

    assert_eq!(server.remaining().await, 0);
}

#[tokio::test]
async fn test_anthropic_tool_call() {
    let script = vec![MockResponse::tool_call(
        "call_1",
        "Bash",
        json!({"command": "ls"}),
    )];
    let server = MockLlmServer::start(script).await.unwrap();

    let client = Client::builder().no_proxy().build().unwrap();
    let res = client
        .post(format!("{}/v1/messages", server.url()))
        .send()
        .await
        .unwrap();
    assert!(res.status().is_success());

    let mut stream = res.bytes_stream().eventsource();
    let mut events = Vec::new();
    while let Some(Ok(event)) = stream.next().await {
        events.push((event.event, event.data));
    }

    assert_eq!(events[1].0, "content_block_start");
    assert!(events[1].1.contains("call_1"));
    assert!(events[1].1.contains("Bash"));
    assert_eq!(events[2].0, "content_block_delta");
    assert!(events[2].1.contains("ls"));
    assert_eq!(events[3].0, "content_block_stop");
    assert_eq!(events[4].0, "message_delta");
    assert!(events[4].1.contains("tool_use"));
}

#[tokio::test]
async fn test_openai_tool_call() {
    let script = vec![MockResponse::tool_call(
        "call_1",
        "Bash",
        json!({"command": "ls"}),
    )];
    let server = MockLlmServer::start(script).await.unwrap();

    let client = Client::builder().no_proxy().build().unwrap();
    let res = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .send()
        .await
        .unwrap();
    assert!(res.status().is_success());

    let mut stream = res.bytes_stream().eventsource();
    let mut data = Vec::new();
    while let Some(Ok(event)) = stream.next().await {
        data.push(event.data);
    }

    assert!(
        data.iter()
            .any(|d| d.contains("tool_calls") && d.contains("call_1"))
    );
    assert!(data.iter().any(|d| d.contains("tool_calls")));
    assert!(data.iter().any(|d| d.contains("ls")));
    assert!(
        data.iter()
            .any(|d| d.contains("finish_reason\":\"tool_calls"))
    );
}

#[tokio::test]
async fn test_anthropic_mixed_response() {
    let script = vec![MockResponse::text_then_tool(
        "I will check the files.",
        "call_1",
        "Bash",
        json!({"command": "ls"}),
    )];
    let server = MockLlmServer::start(script).await.unwrap();

    let client = Client::builder().no_proxy().build().unwrap();
    let res = client
        .post(format!("{}/v1/messages", server.url()))
        .send()
        .await
        .unwrap();
    let mut stream = res.bytes_stream().eventsource();
    let mut total_blocks = 0;
    while let Some(Ok(event)) = stream.next().await {
        if event.event == "content_block_start" {
            total_blocks += 1;
        }
    }
    assert_eq!(total_blocks, 2);
}

#[tokio::test]
async fn test_anthropic_error() {
    let script = vec![MockResponse::error(429, "rate limited")];
    let server = MockLlmServer::start(script).await.unwrap();

    let client = Client::builder().no_proxy().build().unwrap();
    let res = client
        .post(format!("{}/v1/messages", server.url()))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 429);
    let text = res.text().await.unwrap();
    assert!(text.contains("rate limited"));
}

#[tokio::test]
async fn test_openai_multi_tool() {
    let script = vec![MockResponse::multi_tool(vec![
        ("call_1".into(), "Bash".into(), json!({"command": "ls"})),
        ("call_2".into(), "Bash".into(), json!({"command": "pwd"})),
    ])];
    let server = MockLlmServer::start(script).await.unwrap();

    let client = Client::builder().no_proxy().build().unwrap();
    let res = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .send()
        .await
        .unwrap();
    assert!(res.status().is_success());
}

#[tokio::test]
async fn test_empty_script_returns_500() {
    let server = MockLlmServer::start(vec![]).await.unwrap();
    let client = Client::builder().no_proxy().build().unwrap();
    let res = client
        .post(format!("{}/v1/messages", server.url()))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 500);
}

#[tokio::test]
async fn test_script_from_yaml_file() {
    let script =
        mock_llm_server::MockResponse::load_script_from_yaml("fixtures/demo_scenario.yaml")
            .await
            .unwrap();
    assert_eq!(script.len(), 3);

    let server = MockLlmServer::start(script).await.unwrap();
    let client = Client::builder().no_proxy().build().unwrap();
    let res1 = client
        .post(format!("{}/v1/messages", server.url()))
        .send()
        .await
        .unwrap();
    assert!(res1.status().is_success());
    let res2 = client
        .post(format!("{}/v1/messages", server.url()))
        .send()
        .await
        .unwrap();
    assert!(res2.status().is_success());
    let res3 = client
        .post(format!("{}/v1/messages", server.url()))
        .send()
        .await
        .unwrap();
    assert_eq!(res3.status().as_u16(), 400);
    assert_eq!(server.remaining().await, 0);
}

#[tokio::test]
async fn test_script_from_dir() {
    let script = mock_llm_server::MockResponse::load_script_from_dir("tests/fixtures/demo_dir").await.unwrap();
    assert_eq!(script.len(), 2);

    let server = MockLlmServer::start(script).await.unwrap();
    let client = Client::builder().no_proxy().build().unwrap();

    // First request should be successful text response
    let res1 = client
        .post(format!("{}/v1/messages", server.url()))
        .send()
        .await
        .unwrap();
    assert!(res1.status().is_success());

    // Second request should fail with 500
    let res2 = client
        .post(format!("{}/v1/messages", server.url()))
        .send()
        .await
        .unwrap();
    assert_eq!(res2.status().as_u16(), 500);
    let text = res2.text().await.unwrap();
    assert!(text.contains("Something went wrong manually"));

    assert_eq!(server.remaining().await, 0);
}
