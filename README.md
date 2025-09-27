# ShredStreamGrpc

A gRPC client implementation for subscribing to and processing Solana transaction data.

## Features

- Real-time subscription to Solana transaction data via gRPC
- Support for processing transaction entries and transactions
- Asynchronous transaction data processing
- Custom callback function support for transaction events
- Built-in error handling mechanism

## Installation

Add the following to your `Cargo.toml`:

```toml
[dependencies]
shred-parsed = { path = ".", version = "0.1.0" }
```

## Usage Examples

### 1. Initializing the Client

```rust
use shred_parsed::ShredStreamGrpc;

async fn setup_client() -> Result<ShredStreamGrpc, Box<dyn std::error::Error>> {
    let endpoint = "http://127.0.0.1:10000";
    let client = ShredStreamGrpc::new(endpoint.to_string()).await?;
    Ok(client)
}
```

### 2. Subscribing to Transaction Data

```rust
use shred_parsed::PumpfunEvent;
use solana_sdk::pubkey::Pubkey;

async fn subscribe_to_transactions() -> Result<(), Box<dyn std::error::Error>> {
    let client = ShredStreamGrpc::new("http://127.0.0.1:10000".to_string()).await?;
    
    let callback = |event: PumpfunEvent| {
        match event {
            PumpfunEvent::NewToken(token_info) => {
                println!("New token created: {:?}", token_info);
            },
            PumpfunEvent::NewDevTrade(trade_info) => {
                println!("Dev trade executed: {:?}", trade_info);
            },
            PumpfunEvent::NewUserTrade(trade_info) => {
                println!("User trade executed: {:?}", trade_info);
            },
            PumpfunEvent::NewBotTrade(trade_info) => {
                println!("Bot trade executed: {:?}", trade_info);
            },
            PumpfunEvent::Error(err) => {
                eprintln!("Error occurred: {}", err);
            }
        }
    };

    // Optional: Specify bot wallet address for filtering
    let bot_wallet = Some(Pubkey::new_unique());
    client.shredstream_subscribe(callback, bot_wallet).await?;
    
    Ok(())
}
```

## Error Handling

```rust
use shred_parsed::AnyResult;

async fn handle_errors() {
    match ShredStreamGrpc::new("http://127.0.0.1:10000".to_string()).await {
        Ok(client) => {
            println!("Client initialized successfully");
        },
        Err(e) => {
            eprintln!("Failed to initialize client: {}", e);
        }
    }
}
```

## Important Notes

- Ensure the gRPC server address is correct and accessible
- Callback functions should be thread-safe (Send + Sync)
- Implement appropriate error retry mechanisms in production environments
- Be mindful of memory usage when processing large volumes of transaction data

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.


### Telegram group:
https://t.me/fnzero_group
