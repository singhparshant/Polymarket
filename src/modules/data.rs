use polymarket_rs_client::ClobClient;
use serde_json;
use std::{collections::HashMap, env, sync::Arc};
use tokio::sync::Mutex;

use crate::modules::types::PositionApiResponse;

/// Fetch current positions from Polymarket API using the /positions endpoint
/// Returns a map of token_id -> quantity for the specified tokens
pub async fn fetch_current_positions(
    client: &Arc<Mutex<ClobClient>>,
    assets_id: &String,
    condition_id: &String,
) -> Result<HashMap<String, f64>, Box<dyn std::error::Error>> {
    let _ = client; // not used currently; kept for parity with signature
    let mut positions = HashMap::new();

    // Initialize with zero positions
    // for token_id in token_ids {
    // positions.insert(token_id.clone(), 0.0);
    // }
    positions.insert(assets_id.clone(), 0.0);

    // Prefer address from client; fallback to env PROXYWALLET if provided
    let user_address = env::var("PROXYWALLET")
        .unwrap_or_else(|_| "0x750fe60b6d834ba2957390c560c5df82ccdc7a84".to_string());
    println!("Fetching positions for user: {}", user_address);

    // Use the /positions endpoint to get actual positions
    let positions_url = "https://data-api.polymarket.com/positions";
    let query_params = vec![("user", user_address), ("market", condition_id.to_string())];

    // Make direct HTTP request to positions endpoint
    let http_client = reqwest::Client::new();
    match http_client
        .get(positions_url)
        .query(&query_params)
        .send()
        .await
    {
        Ok(response) => {
            if response.status().is_success() {
                let positions_data: Vec<PositionApiResponse> = response.json().await?;
                for position in positions_data {
                    if *assets_id == position.asset {
                        positions.insert(position.asset, position.size);
                    }
                }
            }
        }
        Err(e) => {
            println!("Error calling positions API: {:?}", e);
        }
    }

    Ok(positions)
}

/// Fetch open orders from Polymarket API
/// Returns a list of open orders for debugging purposes
pub async fn fetch_open_orders(
    client: &Arc<Mutex<ClobClient>>,
    _token_ids: &[String],
) -> Result<Vec<serde_json::Value>, Box<dyn std::error::Error>> {
    let _ = client; // Temporarily disabled due to upstream client query bug
    println!("Info: fetch_open_orders disabled; relying on user_ws for open order state.");
    Ok(Vec::new())
}
