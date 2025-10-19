use mongodb::{
    bson::{doc, Document},
    Client as MongoClient, Collection,
};
use reqwest::Client;
use serde_json::Value;
use std::collections::HashMap;
// chrono imports removed since we use string dates for MongoDB

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
            "$gt": "2025-10-21T00:00:00Z",
            "$lt": "2025-10-31T23:59:59Z"
        },
        "enableNegRisk": true,
        "negRiskMarketID": "0x7971728ccf4866d610621f7b8af18b78c1649e6f2c28ce2073d54af61be42600"
    };

    // Find matching documents
    let mut cursor = collection.find(filter).await?;
    let mut results = Vec::new();

    while cursor.advance().await? {
        let doc = cursor.deserialize_current()?;
        results.push(doc);
    }

    println!("Found {} events matching criteria", results.len());
    let mut token_ids = Vec::new();

    if !results.is_empty() {
        if let Some(markets) = results[0].get("markets").and_then(|m| m.as_array()) {
            for (i, market) in markets.iter().enumerate() {
                if let Some(market_obj) = market.as_document() {
                    if let Some(clob_token_ids_str) =
                        market_obj.get("clobTokenIds").and_then(|c| c.as_str())
                    {
                        // Parse the JSON string
                        if let Ok(parsed_array) =
                            serde_json::from_str::<Vec<String>>(clob_token_ids_str)
                        {
                            token_ids.push(parsed_array[0].clone());
                        } else {
                            println!("Failed to parse clobTokenIds JSON string");
                        }
                    } else {
                        println!("No clobTokenIds found in market {}", i);
                    }
                }
            }
        }
        // println!("Prices: {:?}", prices);
        let client = Client::new();
        loop {
            let prices = get_prices(&client, token_ids.clone()).await?;
            let sum_prices_sell = prices.values().sum::<f64>();
            println!("Total prices sell: {}", sum_prices_sell);
            if sum_prices_sell < 1.0 {
                print!("ARBITRAGE DETECTED: {:?} ", sum_prices_sell);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    }
    Ok(results)
}

async fn get_prices(
    client: &Client,
    clob_token_ids: Vec<String>,
) -> Result<HashMap<String, f64>, Box<dyn std::error::Error>> {
    let url = "https://clob.polymarket.com/prices";

    // Build request body as array of {token_id, side} objects
    let mut request_body = Vec::new();
    for token_id in &clob_token_ids {
        request_body.push(serde_json::json!({
            "token_id": token_id,
            "side": "SELL"
        }));
    }

    let response = client.post(url).json(&request_body).send().await?;
    println!("Response status: {}", response.status());
    let text = response.text().await?;
    let json: Value = serde_json::from_str(&text)?;

    let mut prices: HashMap<String, f64> = HashMap::new();

    if let Some(prices_obj) = json.as_object() {
        for (token_id, price_data) in prices_obj {
            println!("Token ID: {:?}", token_id);
            println!("Price Data: {:?}", price_data);
            if let Some(price_obj) = price_data.as_object() {
                let sell_price = price_obj
                    .get("SELL")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.0);

                prices.insert(token_id.clone(), sell_price);
            }
        }
    }

    Ok(prices)
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
