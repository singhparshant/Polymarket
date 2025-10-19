use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
struct NonceResponse {
    nonce: String,
}

#[tokio::main]
pub async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new();
    let response = client
        .get("https://gamma-api.polymarket.com/nonce")
        .send()
        .await?;

    // --- Print Response Details ---

    // 1. Print the status and HTTP version
    println!("Status: {}", response.status());
    println!("HTTP Version: {:?}", response.version());
    println!("---");

    // 2. Print the headers
    // The `headers()` method returns a HeaderMap, which can be printed nicely
    // using the debug formatter `"{:#?}"`.
    println!("Headers:\n{:#?}", response.headers());
    println!("---");

    // 3. Get the body as text and print it
    // Note: `.text()` consumes the response body, so you should call it
    // after you're done inspecting status and headers.
    let body = response.text().await?;
    println!("Body:\n{}", body);

    // let response_text = response.text().await?;

    // println!("reponse: {}", response_text);
    // let nonce_json: Value = serde_json::from_str(&response_text)?;
    // println!("{}", serde_json::to_string_pretty(&nonce_json)?);

    // // Access the nonce field safely
    // if let Some(nonce) = nonce_json.get("nonce") {
    //     println!("Nonce: {}", nonce);
    // }

    Ok(())
}
