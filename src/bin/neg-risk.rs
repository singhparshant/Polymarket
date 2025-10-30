use futures_util::{SinkExt, StreamExt};
use mongodb::{
    bson::{doc, Document},
    Client as MongoClient, Collection,
};
use reqwest::Client;
use serde_json::{json, Value};
use std::collections::HashMap;
use tokio_tungstenite::{connect_async, tungstenite::Message};
// Copy the parsing logic from websocket.rs and types.rs directly here

#[derive(Debug, Clone)]
struct MarketUpdate {
    asset_id: String,
    best_bid: f64,
    best_ask: f64,
    ts: i64,
}

#[derive(Debug, serde::Deserialize)]
struct OrderSummary {
    price: String,
    size: String,
}

#[derive(Debug, serde::Deserialize)]
struct BookMessage {
    event_type: String,
    asset_id: String,
    market: String,
    bids: Vec<OrderSummary>,
    asks: Vec<OrderSummary>,
    timestamp: String,
    hash: String,
}

#[derive(Debug, serde::Deserialize)]
struct PriceChange {
    asset_id: String,
    price: String,
    side: String,
    size: String,
    hash: String,
    best_bid: String,
    best_ask: String,
}

#[derive(Debug, serde::Deserialize)]
struct PriceChangeMessage {
    price_changes: Vec<PriceChange>,
    event_type: String,
    market: String,
    timestamp: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(untagged)]
enum MarketWebSocketMessages {
    BookMessage(BookMessage),
    PriceChangeMessage(PriceChangeMessage),
}

fn parse_update(txt: &str) -> Vec<MarketUpdate> {
    let mut out = Vec::new();

    // Try array first: market WS may send batches like `[ {...}, {...} ]`
    if let Ok(msgs) = serde_json::from_str::<Vec<MarketWebSocketMessages>>(txt) {
        println!("Parsed array of MarketWebSocketMessages",);
        for msg in msgs {
            push_updates_from_msg(&mut out, msg);
        }
        return out;
    }

    // Fallback to single message
    match serde_json::from_str::<MarketWebSocketMessages>(txt) {
        Ok(msg) => {
            // println!("Parsed single MarketWebSocketMessage",);
            push_updates_from_msg(&mut out, msg);
        }
        Err(_) => {
            // Try single BookMessage
            if let Ok(book) = serde_json::from_str::<BookMessage>(txt) {
                push_updates_from_msg(&mut out, MarketWebSocketMessages::BookMessage(book));
            }
        }
    }

    out
}

fn push_updates_from_msg(out: &mut Vec<MarketUpdate>, msg: MarketWebSocketMessages) {
    match msg {
        MarketWebSocketMessages::BookMessage(book) => {
            let asset_id = book.asset_id;
            let parse = |s: &str| {
                let t = if s.starts_with('.') {
                    format!("0{}", s)
                } else {
                    s.to_string()
                };
                t.parse::<f64>().ok()
            };
            let best_bid = book
                .bids
                .last()
                .and_then(|b| parse(&b.price))
                .unwrap_or(0.0);
            let best_ask = book
                .asks
                .last()
                .and_then(|a| parse(&a.price))
                .unwrap_or(1.0);
            let ts = book.timestamp.parse::<i64>().unwrap_or(0);
            out.push(MarketUpdate {
                asset_id,
                best_bid,
                best_ask,
                ts,
            });
        }
        MarketWebSocketMessages::PriceChangeMessage(price_change_message) => {
            for change in price_change_message.price_changes {
                let asset_id = change.asset_id;
                let best_bid = change.best_bid.parse::<f64>().unwrap_or(0.0);
                let best_ask = change.best_ask.parse::<f64>().unwrap_or(1.0);
                let ts = price_change_message.timestamp.parse::<i64>().unwrap_or(0);
                out.push(MarketUpdate {
                    asset_id,
                    best_bid,
                    best_ask,
                    ts,
                });
            }
        }
    }
}

pub async fn get_neg_risk_markets() -> Result<(), Box<dyn std::error::Error>> {
    // MongoDB setup
    let mongo_uri =
        std::env::var("MONGODB_URI").unwrap_or_else(|_| "mongodb://localhost:27017".to_string());
    let mongo_client = MongoClient::with_uri_str(&mongo_uri).await?;
    let database = mongo_client.database("polymarket");
    let collection: Collection<Document> = database.collection("all_events");

    let client = Client::new();
    let url = "https://gamma-api.polymarket.com/events";
    let mut last_offset = 0;
    let mut batch: Vec<Document> = Vec::new();
    const BATCH_SIZE: usize = 1000;

    loop {
        let url = if last_offset > 0 {
            format!("{}?limit=100&ascending=true&offset={}", url, last_offset)
        } else {
            format!("{}?limit=100&ascending=true", url)
        };

        let url = reqwest::Url::parse(&url)?;
        let response = client.get(url).send().await?;
        let text = response.text().await?;
        let json: Value = serde_json::from_str(&text)?;

        // Collect events in batch
        if let Some(data_array) = json.as_array() {
            for event in data_array {
                let doc: Document = serde_json::from_value(event.clone())?;
                batch.push(doc);

                // Store batch when it reaches 1000 records
                if batch.len() >= BATCH_SIZE {
                    let insert_result = collection.insert_many(&batch).await?;
                    println!(
                        "Inserted {} documents with _ids:",
                        insert_result.inserted_ids.len()
                    );
                    batch.clear(); // Clear the batch after storing
                }
            }
        }

        println!(
            "Collected events at offset: {} (batch size: {})",
            last_offset,
            batch.len()
        );

        if json.as_array().map_or(0, |arr| arr.len()) < 100 {
            break;
        }

        last_offset += 100;
        std::thread::sleep(std::time::Duration::from_millis(500));
    }

    // Store any remaining events in the final batch
    if !batch.is_empty() {
        let insert_result = collection.insert_many(&batch).await?;
        println!(
            "Inserted final {} documents with _ids:",
            insert_result.inserted_ids.len()
        );
        for (_key, value) in &insert_result.inserted_ids {
            println!("{}", value);
        }
    }

    Ok(())
}

pub async fn filter_events_by_date_range() -> Result<Vec<Document>, Box<dyn std::error::Error>> {
    // MongoDB setup
    let mongo_uri =
        std::env::var("MONGODB_URI").unwrap_or_else(|_| "mongodb://localhost:27017".to_string());
    let mongo_client = MongoClient::with_uri_str(&mongo_uri).await?;
    let database = mongo_client.database("polymarket");
    let collection: Collection<Document> = database.collection("all_events");

    // Date range: Oct 21, 2025 to Oct 31, 2025

    // Create filter query using string dates for MongoDB comparison
    let filter = doc! {
        "active": true,
        "endDate": {
            "$gt": "2025-10-25T00:00:00Z",
            "$lt": "2025-10-31T23:59:59Z"
        },
        "enableNegRisk": true,
        "volume": {
            "$gt": 0
        }
    };

    // Find matching documents
    let mut cursor = collection.find(filter).await?;
    let mut results = Vec::new();

    let mut token_ids_map: HashMap<String, Vec<String>> = HashMap::new();
    let mut token_ids = Vec::new();

    while cursor.advance().await? {
        let doc = cursor.deserialize_current()?;
        results.push(doc);
    }
    println!("Found {} events matching criteria", results.len());

    for event in &results {
        let neg_risk_market_id = event.get("negRiskMarketID").and_then(|n| n.as_str());

        if let Some(markets) = event.get("markets").and_then(|m| m.as_array()) {
            for market in markets {
                if let Some(market_obj) = market.as_document() {
                    // Skip closed markets
                    if market_obj
                        .get("closed")
                        .and_then(|c| c.as_bool())
                        .unwrap_or(false)
                    {
                        continue;
                    }

                    // Extract token IDs
                    if let Some(clob_token_ids_str) =
                        market_obj.get("clobTokenIds").and_then(|c| c.as_str())
                    {
                        if let Ok(parsed_array) =
                            serde_json::from_str::<Vec<String>>(clob_token_ids_str)
                        {
                            token_ids.push(parsed_array[0].clone());

                            // Group by negRiskMarketID if it exists
                            if let Some(neg_risk_id) = neg_risk_market_id {
                                token_ids_map
                                    .entry(neg_risk_id.to_string())
                                    .or_insert(Vec::new())
                                    .push(parsed_array[0].clone());
                            }
                        } else {
                            println!("Failed to parse clobTokenIds JSON string");
                        }
                    }
                }
            }
        }
    }

    // println!("Token IDs: {}", token_ids.len());
    println!("Token IDs Map length: {:?}", token_ids_map.keys().len());

    // Flatten all token IDs from the HashMap into a single vector
    let mut all_token_ids = Vec::new();
    for (neg_risk_id, token_list) in &token_ids_map {
        all_token_ids.extend(token_list.clone());
    }

    println!("Total token IDs: {}", all_token_ids.len());

    // Initialize price tracking: negRiskMarketID -> HashMap<token_id, price>
    let mut price_tracker: HashMap<String, HashMap<String, f64>> = HashMap::new();
    for (neg_risk_id, _) in &token_ids_map {
        price_tracker.insert(neg_risk_id.clone(), HashMap::new());
    }

    // Get initial prices using REST API
    let client = Client::new();
    println!("Fetching initial prices...");
    let initial_prices = get_prices(&client, all_token_ids.clone()).await?;

    // Populate the price tracker with initial prices
    for (neg_risk_id, token_list) in &token_ids_map {
        if let Some(price_map) = price_tracker.get_mut(neg_risk_id) {
            for token_id in token_list {
                if let Some(price) = initial_prices.get(token_id) {
                    price_map.insert(token_id.clone(), *price);
                }
            }
        }
    }

    // Print initial state
    for (neg_risk_id, price_map) in &price_tracker {
        let sum: f64 = price_map.values().sum();
        println!(
            "NegRiskMarketID {}: {} tokens, sum = {:.4}",
            neg_risk_id,
            price_map.len(),
            sum
        );
        if sum < 1.0 && sum > 0.0 {
            println!(
                "🚨 INITIAL ARBITRAGE DETECTED in NegRiskMarketID {}: {:.4}",
                neg_risk_id, sum
            );
        }
    }
    // loop {
    //     // Fetch prices for all token IDs at once
    //     let start_time = Instant::now();
    //     let prices = get_prices(&client, all_token_ids.clone()).await?;

    //     let duration = start_time.elapsed().as_millis();
    //     println!("Duration: {}ms", duration);
    //     // Group back by negRiskMarketID and sum prices for each group
    //     for (neg_risk_id, token_list) in &token_ids_map {
    //         let mut group_sum = 0.0;
    //         for token_id in token_list {
    //             if let Some(price) = prices.get(token_id) {
    //                 group_sum += price;
    //             }
    //         }
    //         // println!(
    //         //     "NegRiskMarketID {}: Total price = {}",
    //         //     neg_risk_id, group_sum
    //         // );

    //         // Check for arbitrage in this specific group
    //         if group_sum < 1.0 && group_sum > 0.0 {
    //             println!(
    //                 "{}🚨 ARBITRAGE DETECTED in NegRiskMarketID {}: {}",
    //                 chrono::Utc::now().format("%Y-%m-%d %H:%M:%S"),
    //                 neg_risk_id,
    //                 group_sum
    //             );
    //         }
    //     }

    //     // Also check overall arbitrage
    //     // let total_sum = prices.values().sum::<f64>();
    //     // println!("Total prices sell: {}", total_sum);
    //     // if total_sum < 1.0 {
    //     //     println!("🚨 OVERALL ARBITRAGE DETECTED: {}", total_sum);
    //     //     break;
    //     // }
    //     std::thread::sleep(std::time::Duration::from_millis(100));
    // }

    const WS_MARKET_URL: &str = "wss://ws-subscriptions-clob.polymarket.com/ws/market";
    match connect_async(&WS_MARKET_URL.to_string()).await {
        Ok((mut ws, _)) => {
            println!("Connected to websocket");
            let sub_msg = json!({"assets_ids": all_token_ids});
            let res = ws.send(Message::Text(sub_msg.to_string().into())).await;
            if res.is_err() {
                println!("Error sending market subscription: {:?}", res);
                return Ok(results);
            }
            println!("Sent market subscription payload");

            // Listen for price updates
            loop {
                match ws.next().await {
                    Some(Ok(msg)) => {
                        match msg {
                            Message::Text(text) => {
                                // Use the existing parse_update function from websocket.rs
                                let updates = parse_update(&text);

                                // if updates.len() > 0 {
                                //     println!("UPDATES: {:?}", updates);
                                // }
                                for update in updates {
                                    let asset_id = &update.asset_id;
                                    let best_ask = update.best_ask; // Use best_ask as sell price

                                    // Find which negRiskMarketID this token belongs to and update price
                                    for (neg_risk_id, token_list) in &token_ids_map {
                                        if token_list.contains(asset_id) {
                                            // Update the price in our tracker
                                            if let Some(price_map) =
                                                price_tracker.get_mut(neg_risk_id)
                                            {
                                                let old_price = price_map.get(asset_id).copied();
                                                price_map.insert(asset_id.clone(), best_ask);

                                                // Check if price actually changed
                                                if old_price.map_or(true, |old| {
                                                    (old - best_ask).abs() > 0.0001
                                                }) {
                                                    // Calculate new sum for this negRiskMarketID
                                                    let sum: f64 = price_map.values().sum();

                                                    // println!("Price update for {} in NegRiskMarketID {}: {:.4} -> {:.4}, sum = {:.4}",
                                                    //     asset_id, neg_risk_id,
                                                    //     old_price.unwrap_or(0.0), best_ask, sum);

                                                    // Check for arbitrage in this specific group
                                                    if sum < 1.0 && sum > 0.0 {
                                                        println!("🚨 ARBITRAGE DETECTED in NegRiskMarketID {}: {:.4}", 
                                                            neg_risk_id, sum);
                                                    }
                                                }
                                            }
                                            break;
                                        }
                                    }
                                }
                            }
                            Message::Close(_) => {
                                println!("WebSocket connection closed");
                                break;
                            }
                            _ => {}
                        }
                    }
                    Some(Err(e)) => {
                        println!("WebSocket error: {:?}", e);
                        break;
                    }
                    None => {
                        println!("WebSocket connection ended");
                        break;
                    }
                }
            }
        }
        Err(e) => {
            println!("Error connecting to websocket: {:?}", e);
        }
    }

    Ok(results)
}

async fn get_prices(
    client: &Client,
    clob_token_ids: Vec<String>,
) -> Result<HashMap<String, f64>, Box<dyn std::error::Error>> {
    let url = "https://clob.polymarket.com/prices";
    let mut all_prices: HashMap<String, f64> = HashMap::new();

    // Process in batches of 50 to avoid payload limit
    const BATCH_SIZE: usize = 50;

    for chunk in clob_token_ids.chunks(BATCH_SIZE) {
        // Build request body as array of {token_id, side} objects
        let mut request_body = Vec::new();
        for token_id in chunk {
            request_body.push(serde_json::json!({
                "token_id": token_id,
                "side": "SELL"
            }));
        }

        let response = client.post(url).json(&request_body).send().await?;
        // println!("Response status: {}", response.status());
        if response.status().is_server_error() || response.status().is_client_error() {
            println!("Error fetching prices: {}", response.status());
            continue;
        }
        let text = response.text().await?;
        let json: Value = serde_json::from_str(&text)?;

        if let Some(prices_obj) = json.as_object() {
            for (token_id, price_data) in prices_obj {
                if let Some(price_obj) = price_data.as_object() {
                    let sell_price = price_obj
                        .get("SELL")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse::<f64>().ok())
                        .unwrap_or(0.0);

                    all_prices.insert(token_id.clone(), sell_price);
                }
            }
        }

        // Small delay between batches to be respectful to the API
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    Ok(all_prices)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Uncomment to fetch and store events
    // get_neg_risk_markets().await?;

    // Filter events by date range
    let filtered_events = filter_events_by_date_range().await?;
    println!("Total filtered events: {}", filtered_events.len());

    Ok(())
}
