use alloy::{
    primitives::{Address, Bytes, U256},
    providers::{Provider, ProviderBuilder},
    signers::Signer,
    sol,
    sol_types::SolCall,
};
use alloy_primitives::{address, B256};
use alloy_signer_local::PrivateKeySigner;
use anyhow::Result;
use std::env;

// ============================================================================
// CONSTANTS AND TYPES
// ============================================================================

// Addresses (same as your JS constants)
const CONDITIONAL_TOKENS_FRAMEWORK_ADDRESS: Address =
    address!("0x4D97DCd97eC945f40cF65F87097ACe5EA0476045");
const USDC_ADDRESS: Address = address!("0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174");
const NEG_RISK_ADAPTER_ADDRESS: Address = address!("0xd91E80cF2E7be2e162c6513ceD06f1dD0dA35296");
const USDCE_DIGITS: u32 = 6; // USDC has 6 decimals

// Operation types (like your JS OperationType enum)
#[derive(Debug, Clone, Copy)]
enum OperationType {
    Call = 0,
    DelegateCall = 1,
}

// Safe transaction structure (like your JS SafeTransaction interface)
#[derive(Debug, Clone)]
struct SafeTransaction {
    to: Address,
    data: Bytes,
    operation: OperationType,
    value: U256,
}

// Transaction type enum
#[derive(Debug, Clone, Copy)]
pub enum TransactionType {
    Split,
    Merge,
}

// ============================================================================
// SMART CONTRACT DEFINITIONS
// ============================================================================

sol! {
    // Safe contract for executing transactions
    #[sol(rpc)]
    contract Safe {
        function execTransaction(
            address to,
            uint256 value,
            bytes data,
            uint8 operation,
            uint256 safeTxGas,
            uint256 baseGas,
            uint256 gasPrice,
            address gasToken,
            address refundReceiver,
            bytes signatures
        ) external payable returns (bool success);

        function getTransactionHash(
            address to,
            uint256 value,
            bytes data,
            uint8 operation,
            uint256 safeTxGas,
            uint256 baseGas,
            uint256 gasPrice,
            address gasToken,
            address refundReceiver,
            uint256 nonce
        ) external view returns (bytes32);

        function nonce() external view returns (uint256);
    }

    // Conditional tokens contract for the split/merge function
    contract ConditionalTokens {
        function splitPosition(
            address collateralToken,
            bytes32 parentCollectionId,
            bytes32 conditionId,
            uint256[] calldata partition,
            uint256 amount
        ) external;
        function mergePositions(
            address collateralToken,
            bytes32 parentCollectionId,
            bytes32 conditionId,
            uint256[] calldata partition,
            uint256 amount
        ) external;
    }
}

pub async fn sign_safe_hash_eth_sign(signer: &PrivateKeySigner, tx_hash: B256) -> Result<Bytes> {
    // EIP-191: prefixes before signing
    let sig = signer.sign_message(tx_hash.as_slice()).await?;
    let mut sig_bytes = sig.as_bytes().to_vec();

    // Normalize v to 31/32 for Safe "eth_sign" signatures
    let v = match sig_bytes[64] {
        0 | 1 => sig_bytes[64] + 31,
        27 | 28 => sig_bytes[64] + 4,
        other => return Err(anyhow::anyhow!("invalid v: {}", other)),
    };
    sig_bytes[64] = v;

    Ok(Bytes::from(sig_bytes))
}

// ============================================================================
// SAFE TRANSACTION EXECUTION
// ============================================================================

// Sign and execute Safe transaction (like your JS signAndExecuteSafeTransaction)
async fn sign_and_execute_safe_transaction<P>(
    signer: &PrivateKeySigner,
    provider: P,
    safe_address: Address,
    txn: SafeTransaction,
    gas_price: Option<u128>,
) -> Result<B256>
where
    P: Provider + Clone,
{
    // Create Safe contract instance
    let safe = Safe::new(safe_address, &provider);

    // Get current nonce (like your JS safe.nonce())
    let nonce = safe.nonce().call().await?;

    // Safe transaction parameters (like your JS)
    let safe_tx_gas = U256::ZERO;
    let base_gas = U256::ZERO;
    let gas_price_u256 = U256::ZERO; //gas_price.map(U256::from).unwrap_or(U256::ZERO);
    let gas_token = Address::ZERO;
    let refund_receiver = Address::ZERO;

    // Get transaction hash (like your JS safe.getTransactionHash())
    let tx_hash = safe
        .getTransactionHash(
            txn.to,
            txn.value,
            txn.data.clone(),
            txn.operation as u8,
            safe_tx_gas,
            base_gas,
            gas_price_u256,
            gas_token,
            refund_receiver,
            nonce,
        )
        .call()
        .await?;

    // CRITICAL: Use sign_hash to sign the raw hash, NOT sign_message.
    // This method does not add the EIP-191 prefix.
    let signature = sign_safe_hash_eth_sign(signer, tx_hash).await?;

    // Use the signature directly (it's already Bytes)
    let packed_sig = signature;

    // Execute the transaction (like your JS safe.execTransaction())
    let tx = safe.execTransaction(
        txn.to,
        txn.value,
        txn.data,
        txn.operation as u8,
        safe_tx_gas,
        base_gas,
        gas_price_u256,
        gas_token,
        refund_receiver,
        packed_sig,
    );

    // Add gas price if specified
    let tx = if let Some(gas_price) = gas_price {
        tx.gas_price(gas_price)
    } else {
        tx
    };

    let pending_tx = tx.send().await?;
    println!("Transaction sent: {:?}", pending_tx.tx_hash());
    let receipt = pending_tx.get_receipt().await?;

    println!(
        "Transaction confirmed in block: {:?}",
        receipt.block_number.unwrap_or_default()
    );

    Ok(receipt.transaction_hash)
}

// ============================================================================
// PUBLIC API
// ============================================================================

/// Execute split or merge transaction
pub async fn execute_split_merge(
    tx_type: TransactionType,
    amount: &str,
    condition_id: &str,
    safe_address: &str,
    neg_risk: bool,
) -> Result<B256> {
    // Setup provider and wallet (like your JS code)
    let rpc_url = env::var("RPC_URL").expect("RPC_URL env var");
    let private_key = env::var("PK").expect("PK env var");

    let signer: PrivateKeySigner = private_key.parse()?;
    let wallet_address = signer.address();
    println!("Address: {}", wallet_address);

    println!("Safe Address: {}", safe_address);

    let provider = ProviderBuilder::new()
        .wallet(signer.clone())
        .connect_http(rpc_url.parse()?);

    // Parse safe address
    let safe_addr: Address = safe_address.parse()?;

    // Convert condition_id string to B256 (like ethers.utils.hexlify in JS)
    let condition_id_bytes = hex::decode(condition_id.trim_start_matches("0x"))?;
    let mut condition_id_array = [0u8; 32];
    condition_id_array.copy_from_slice(&condition_id_bytes);
    let condition_id_b256 = B256::from(condition_id_array);

    // Parse the amount (like ethers.utils.parseUnits in JS)
    let amount_parsed = U256::from(amount.parse::<u64>()?) * U256::from(10_u64.pow(USDCE_DIGITS));

    // Create the function call data based on transaction type
    let call_data = match tx_type {
        TransactionType::Split => {
            let split_call = ConditionalTokens::splitPositionCall {
                collateralToken: USDC_ADDRESS,
                parentCollectionId: B256::ZERO, // Usually zero for root collections
                conditionId: condition_id_b256,
                partition: vec![U256::from(1), U256::from(2)], // YES/NO tokens
                amount: amount_parsed,
            };
            split_call.abi_encode()
        }
        TransactionType::Merge => {
            let merge_call = ConditionalTokens::mergePositionsCall {
                collateralToken: USDC_ADDRESS,
                parentCollectionId: B256::ZERO, // Usually zero for root collections
                conditionId: condition_id_b256,
                partition: vec![U256::from(1), U256::from(2)], // YES/NO tokens
                amount: amount_parsed,
            };
            merge_call.abi_encode()
        }
    };

    println!(
        "{} data: {:?}",
        match tx_type {
            TransactionType::Split => "Split",
            TransactionType::Merge => "Merge",
        },
        hex::encode(&call_data)
    );

    // Determine the target address (like your JS logic)
    let target_address = if neg_risk {
        NEG_RISK_ADAPTER_ADDRESS
    } else {
        CONDITIONAL_TOKENS_FRAMEWORK_ADDRESS
    };

    // Create the Safe transaction (like your JS SafeTransaction)
    let safe_txn = SafeTransaction {
        to: target_address,
        data: call_data.into(),
        operation: OperationType::Call,
        value: U256::ZERO,
    };

    // Sign and execute the Safe transaction (like your JS signAndExecuteSafeTransaction)
    let tx_hash = sign_and_execute_safe_transaction(
        &signer,
        provider.clone(),
        safe_addr,
        safe_txn,
        Some(200_000_000_000u128), // 200 gwei
    )
    .await?;

    println!("Transaction hash: {:?}", tx_hash);
    println!(
        "Successfully {} {} USDC!",
        match tx_type {
            TransactionType::Split => "split",
            TransactionType::Merge => "merged",
        },
        amount
    );

    Ok(tx_hash)
}
