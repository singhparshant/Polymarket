use crate::modules::types::{AppState, Order, Side};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::Duration;

// -------------------- Monitor Task --------------------
pub async fn monitor_task(state: Arc<Mutex<AppState>>) {
    let mut interval = tokio::time::interval(Duration::from_secs(5));

    loop {
        interval.tick().await;

        if is_shutting_down(&state).await {
            break;
        }

        let s = state.lock().await;

        // Display current state
        println!("\n=== BOT STATUS ===");
        println!("Risk Paused: {}", s.risk_paused);
        println!("Shutting Down: {}", s.shutting_down);
        println!("Total Open Orders: {}", s.my_open_orders.len());

        // Show inventory with dollar values
        if !s.inventory.is_empty() {
            println!("\n--- Inventory ---");
            let mut total_dollar_value = 0.0;
            let mut inventory_dollars = Vec::new();

            for (token_id, quantity) in &s.inventory {
                if *quantity != 0.0 {
                    // Get current price for this token
                    if let Some((bid, ask, _)) = s.last_prices.get(token_id) {
                        let current_price = (bid + ask) / 2.0;
                        let dollar_value = quantity * current_price;
                        total_dollar_value += dollar_value.abs();
                        inventory_dollars.push(dollar_value);
                        println!(
                            "Token {}: {:.2} tokens (${:.2})",
                            token_id, quantity, dollar_value
                        );
                    } else {
                        println!("Token {}: {:.2} tokens (price unknown)", token_id, quantity);
                    }
                }
            }

            // Calculate inventory imbalance in dollars
            if inventory_dollars.len() == 2 {
                let imbalance_dollars = (inventory_dollars[0] - inventory_dollars[1]).abs();
                println!(
                    "Inventory Imbalance: ${:.2} (max: ${:.2})",
                    imbalance_dollars, s.max_inventory_imbalance
                );
                println!(
                    "Total Position: ${:.2} (max: ${:.2})",
                    total_dollar_value, s.max_position_size
                );
            }
        }

        // Group orders by token/market
        let mut orders_by_token: std::collections::HashMap<String, Vec<&Order>> =
            std::collections::HashMap::new();
        for order in s.my_open_orders.values() {
            orders_by_token
                .entry(order.asset_id.clone())
                .or_default()
                .push(order);
        }

        if !orders_by_token.is_empty() {
            println!("\n--- Open Orders by Token ---");
            for (token_id, orders) in orders_by_token {
                let buy_count = orders
                    .iter()
                    .filter(|o| matches!(o.side, Side::Buy))
                    .count();
                let sell_count = orders
                    .iter()
                    .filter(|o| matches!(o.side, Side::Sell))
                    .count();
                println!(
                    "Token {}: {} orders ({} buy, {} sell)",
                    token_id,
                    orders.len(),
                    buy_count,
                    sell_count
                );

                // Show order details (first 3 orders per token to avoid spam)
                for (i, order) in orders.iter().take(3).enumerate() {
                    let side_str = match order.side {
                        Side::Buy => "BUY",
                        Side::Sell => "SELL",
                    };
                    println!(
                        "  {}: {} {} @ {:.4} (size: {:.2})",
                        i + 1,
                        side_str,
                        order.id.as_deref().unwrap_or("pending"),
                        order.price,
                        order.size
                    );
                }
                if orders.len() > 3 {
                    println!("  ... and {} more orders", orders.len() - 3);
                }
            }
        }

        // Show last prices
        if !s.last_prices.is_empty() {
            println!("\n--- Last Market Prices ---");
            for (token_id, (bid, ask, _ts)) in &s.last_prices {
                let spread = ask - bid;
                let spread_pct = (spread / bid) * 100.0;
                println!(
                    "Token {}: bid={:.4}, ask={:.4}, spread={:.4} ({:.2}%)",
                    token_id, bid, ask, spread, spread_pct
                );
            }
        }

        println!("==================\n");
    }
}

// -------------------- Helper Functions --------------------
async fn is_shutting_down(state: &Arc<Mutex<AppState>>) -> bool {
    state.lock().await.shutting_down
}
