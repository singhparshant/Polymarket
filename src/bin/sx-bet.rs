use dotenv::dotenv;
use http::Method;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, CONTENT_TYPE, USER_AGENT};
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use std::env;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();
    let client = Client::new();
    let query_params = [
        (
            "marketHashes",
            "0xaaa5ca9f33de8783294b26dc38e943cd9cdb110c1476b72be3a193af2c3ed841",
        ),
        ("baseToken", "0x6629Ce1Cf35Cc1329ebB4F63202F3f197b3F050B"),
    ];
    let url = reqwest::Url::parse_with_params("https://api.sx.bet/orders/", &query_params)?;
    loop {
        let res = client.request(Method::GET, url.clone()).send().await?;
        let text = res.text().await?;
        let json: Value = serde_json::from_str(&text)?;
        println!("{}", serde_json::to_string_pretty(&json)?);

        sleep(Duration::from_secs(2)).await;
    }

    // Ok(()) // Return success
}
