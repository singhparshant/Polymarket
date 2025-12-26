This is a market-making and arbitrage bot for Polymarket written in Rust.
Experimental and not production ready.

## Polymarket Market-Making Bot — High-Level Architecture

### Components

┌───────────────────┐ MarketUpdate ┌────────────────────┐ BotCommand ┌───────────────────┐
│ WebSocket (MKTs) │ ────────────────▶ │ Trading Logic │ ───────────────▶ │ Order Execution │
│ book + price_change│ │ (Strategy Engine) │ │ (REST API) │
└───────────────────┘ └────────────────────┘ └───────────────────┘
│ │ │
▼ ▼ ▼
┌───────────────────┐ ┌────────────────────┐ ┌───────────────────┐
│ User WebSocket │ Inventory │ Shared AppState │ Periodic stats │ Monitor Task │
│ (fills / orders) │ ───────────────▶ │ (Arc<Mutex<>>) │ ───────────────▶ │ (5s display) │
└───────────────────┘ └────────────────────┘ └───────────────────┘
---

### Data Sources

- **Market WebSocket (`websocket.rs`)**
  - `book` snapshots used to derive best bid and ask
  - `price_change` (new schema) with per-asset `best_bid` and `best_ask` for fast deltas

- **User WebSocket (`user_ws.rs`)**
  - Order updates and fills
  - Used to keep inventory in sync

- **REST API (`execution.rs`)**
  - `GET /positions?user=` on startup to seed inventory and persist positions across restarts
  - `get_orders` for optional open-order debugging

---

### Strategy (`trading.rs`)

- **Quote placement**
  - Quotes are anchored to book edges, not the mid-price
  - Bid: 10% below current best bid
  - Ask: 10% above current best ask (scaled toward 1.0)

- **Inventory-based skew**
  - Applied only when dollar imbalance exceeds `MAX_INVENTORY_IMBALANCE`
  - Long YES:
    - Lower ask (sell faster)
    - Lower bid (discourage buying more YES)
  - Short YES:
    - Raise bid (buy faster)
    - Raise ask (discourage selling more YES)

- **Conditional re-quoting**
  - Orders are replaced only if the mid-price bucket changes
  - Bucket definition: `ceil(mid * 100)`
  - Reduces churn on small price movements

- **Risk guards**
  - Cancel all orders and pause if total position value exceeds `MAX_POSITION_SIZE`
  - Pause quoting if prices are extreme:
    - Bid ≤ 0.02
    - Ask ≥ 0.98

---

### Channels

- `market_tx / market_rx`  
  Market WebSocket → Trading Logic  
  Real-time `MarketUpdate`

- `cmd_tx / cmd_rx`  
  Trading Logic → Order Execution  
  `BotCommand::{Create, Cancel, CancelAll, Shutdown}`

---

### Shared State (`Arc<Mutex<AppState>>`)

- `my_open_orders`: active bot orders
- `inventory`: `token_id → quantity`
- `token_pairs / yes_token`: explicit YES/NO mapping  
  (1st / 2nd entries in `ASSETS_IDS`)
- `last_prices`: latest `(bid, ask, timestamp)` per token
- `last_mid_bucket`: `ceil(mid * 100)` per market
- `risk_paused`, `shutting_down`: control flags
- `max_inventory_imbalance`, `max_position_size`: dollar risk limits

---

### Kill Switch

- `Ctrl + C` sets `shutting_down`
- Issues `CancelAll`
- All tasks shut down cleanly
