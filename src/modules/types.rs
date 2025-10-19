use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// -------------------- Domain Types --------------------
// String constants to avoid typos in comparisons/assignments
pub const SIDE_BUY: &str = "BUY";
pub const SIDE_SELL: &str = "SELL";

pub const MSG_PLACEMENT: &str = "PLACEMENT";
pub const MSG_UPDATE: &str = "UPDATE";
pub const MSG_CANCELLATION: &str = "CANCELLATION";

pub const MSG_TRADE: &str = "TRADE";
pub const STATUS_MATCHED: &str = "MATCHED";
pub const STATUS_MINED: &str = "MINED";
pub const STATUS_CONFIRMED: &str = "CONFIRMED";

pub const EVT_BOOK: &str = "book";
pub const EVT_PRICE_CHANGE: &str = "price_change";
pub const EVT_TICK_SIZE_CHANGE: &str = "tick_size_change";
pub const EVT_LAST_TRADE_PRICE: &str = "last_trade_price";
pub type OrderId = String;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Side {
    Buy,
    Sell,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Order {
    pub id: Option<OrderId>,
    pub asset_id: String,
    pub side: Side,
    pub price: f64,
    pub size: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MarketUpdate {
    pub asset_id: String,
    pub best_bid: f64,
    pub best_ask: f64,
    pub ts: i64,
}

/// Commands sent from Trading Logic to Order Execution
/// These represent actionable trading decisions that need to be executed via Polymarket API
#[derive(Clone, Debug)]
pub enum BotCommand {
    /// Place a new order (buy/sell) on the market
    Create(Order),
    /// Cancel a specific order by its ID
    Cancel(OrderId),
    /// Cancel all open orders (risk management)
    CancelAll,
    /// Update inventory when an order gets filled (from external notifications)
    #[allow(dead_code)]
    OrderFilled(OrderId, String, Side, f64, f64), // order_id, token_id, side, price, size
    /// Graceful shutdown signal with chain context (condition_id, proxy_wallet)
    Shutdown(String, String),
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AppState {
    pub my_open_orders: HashMap<OrderId, Order>, // order_id -> order
    pub last_prices: HashMap<String, (f64, f64, i64)>, // asset_id -> (best_bid, best_ask, timestamp)
    // Inventory management
    pub inventory: HashMap<String, f64>, // token_id -> quantity owned
    pub token_pairs: HashMap<String, String>, // yes_token -> no_token mapping
    pub yes_token: Option<String>,       // Explicit YES token ID for reference
    // Risk management
    pub risk_paused: bool,
    pub shutting_down: bool,
    pub max_inventory_imbalance: f64, // max allowed inventory difference between yes/no
    pub max_position_size: f64,       // max total position size per market
    // Quoting management: track last mid-price bucket to reduce cancels
    pub last_mid_bucket: HashMap<String, i32>, // asset_id -> ceil(mid*100) bucket (bucket is an integer between 0 and 100)
}

// -------------------- WebSocket Message Types --------------------

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct OrderSummary {
    pub price: String,
    pub size: String,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct BookMessage {
    pub event_type: String,
    pub asset_id: String,
    pub market: String,
    pub bids: Vec<OrderSummary>,
    pub asks: Vec<OrderSummary>,
    pub timestamp: String,
    pub hash: String,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct PriceChange {
    pub asset_id: String,
    pub price: String,
    pub side: String,
    pub size: String,
    pub hash: String,
    pub best_bid: String,
    pub best_ask: String,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct PriceChangeMessage {
    pub price_changes: Vec<PriceChange>,
    pub event_type: String,
    pub market: String,
    pub timestamp: String,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct TickSizeChangeMessage {
    pub event_type: String,
    pub asset_id: String,
    pub market: String,
    pub old_tick_size: String,
    pub new_tick_size: String,
    pub timestamp: String,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct LastTradePriceMessage {
    pub asset_id: String,
    pub event_type: String,
    pub fee_rate_bps: String,
    pub market: String,
    pub price: String,
    pub side: String,
    pub size: String,
    pub timestamp: String,
}

// -------------------- User WebSocket Message Types --------------------
#[derive(Debug, Deserialize)]
pub struct MakerOrder {
    pub asset_id: String,
    pub matched_amount: String,
    pub order_id: String,
    pub outcome: String,
    pub owner: String,
    pub price: String,
}

#[derive(Debug, Deserialize)]
pub struct UserTradeMessage {
    pub asset_id: String,
    pub event_type: String, // "trade"
    pub id: String,
    pub last_update: String,
    pub maker_orders: Vec<MakerOrder>,
    pub market: String,
    pub matchtime: String,
    pub outcome: String,
    pub owner: String,
    pub price: String,
    pub side: String, // BUY/SELL
    pub size: String,
    pub status: String, // MATCHED/MINED/CONFIRMED/...
    pub taker_order_id: String,
    pub timestamp: String,
    pub trade_owner: String,
    #[serde(rename = "type")]
    pub msg_type: String, // "TRADE"
}

#[derive(Debug, Deserialize)]
pub struct UserOrderMessage {
    pub asset_id: String,
    pub associate_trades: Vec<String>,
    pub event_type: String, // "order"
    pub id: String,         // order id
    pub market: String,
    pub order_owner: String,
    pub original_size: String,
    pub outcome: String,
    pub owner: String,
    pub price: String,
    pub side: String, // BUY/SELL
    pub size_matched: String,
    pub timestamp: String,
    #[serde(rename = "type")]
    pub msg_type: String, // PLACEMENT/UPDATE/CANCELLATION
}

// -------------------- Helper Functions --------------------
pub fn side_str(side: &Side) -> &'static str {
    match side {
        Side::Buy => "BOUGHT",
        Side::Sell => "SOLD",
    }
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum MarketWebSocketMessages {
    BookMessage(BookMessage),
    PriceChangeMessage(PriceChangeMessage),
    TickSizeChangeMessage(TickSizeChangeMessage),
    LastTradePriceMessage(LastTradePriceMessage),
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
// #[serde(tag = "event_type")]
pub enum UserWebSocketMessages {
    // #[serde(rename = "trade")]
    TradeMessage(UserTradeMessage),
    // #[serde(rename = "order")]
    OrderMessage(UserOrderMessage),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PositionApiResponse {
    pub proxyWallet: String, // 0x-prefixed, 40 hex chars
    pub asset: String,
    pub conditionId: String, // 0x-prefixed, 64 hex chars
    pub size: f64,
    pub avgPrice: f64,
    pub initialValue: f64,
    pub currentValue: f64,
    pub cashPnl: f64,
    pub percentPnl: f64,
    pub totalBought: f64,
    pub realizedPnl: f64,
    pub percentRealizedPnl: f64,
    pub curPrice: f64,
    pub redeemable: bool,
    pub mergeable: bool,
    pub title: String,
    pub slug: String,
    pub icon: String,
    pub eventSlug: String,
    pub outcome: String,
    pub outcomeIndex: i32,
    pub oppositeOutcome: String,
    pub oppositeAsset: String,
    pub endDate: String, // could also be chrono::DateTime if ISO 8601
    pub negativeRisk: bool,
}
