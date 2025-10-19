use serde::Serialize;

#[serde_with::skip_serializing_none]
#[derive(Serialize, Default, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SignatureParams<'a> {
    pub payment_token: Option<&'a str>,
    pub payment: Option<&'a str>,
    pub payment_receiver: Option<&'a str>,
    pub gas_price: Option<&'a str>,
    pub operation: Option<&'a str>,
    pub safe_txn_gas: Option<&'a str>,
    pub base_gas: Option<&'a str>,
    pub gas_token: Option<&'a str>,
    pub refund_receiver: Option<&'a str>,
}

impl<'a> SignatureParams<'a> {
    pub fn set_payment_token(&mut self) {
        self.payment_token = Some("0x0000000000000000000000000000000000000000")
    }

    pub fn with_payment_token(mut self) -> Self {
        self.set_payment_token();
        self
    }

    pub fn set_payment(&mut self) {
        self.payment = Some("0");
    }

    pub fn with_payment(mut self) -> Self {
        self.set_payment();
        self
    }

    pub fn set_payment_receiver(&mut self) {
        self.payment_receiver = Some("0x0000000000000000000000000000000000000000")
    }

    pub fn with_payment_receiver(mut self) -> Self {
        self.set_payment_receiver();
        self
    }

    pub fn set_gas_price(&mut self) {
        self.gas_price = Some("0")
    }

    pub fn with_gas_price(mut self) -> Self {
        self.set_gas_price();
        self
    }

    pub fn set_operation(&mut self, operation: &'a str) {
        self.operation = Some(operation)
    }

    pub fn with_operation(mut self, operation: &'a str) -> Self {
        self.set_operation(operation);
        self
    }

    pub fn set_safe_txn_gas(&mut self) {
        self.safe_txn_gas = Some("0")
    }

    pub fn with_safe_txn_gas(mut self) -> Self {
        self.set_safe_txn_gas();
        self
    }

    pub fn set_base_gas(&mut self) {
        self.base_gas = Some("0")
    }

    pub fn with_base_gas(mut self) -> Self {
        self.set_base_gas();
        self
    }

    pub fn set_gas_token(&mut self) {
        self.gas_token = Some("0x0000000000000000000000000000000000000000")
    }

    pub fn with_gas_token(mut self) -> Self {
        self.set_gas_token();
        self
    }

    pub fn set_refund_receiver(&mut self) {
        self.refund_receiver = Some("0x0000000000000000000000000000000000000000")
    }

    pub fn with_refund_receiver(mut self) -> Self {
        self.set_refund_receiver();
        self
    }
}

#[derive(Serialize, Debug)]
pub enum RelayerRequestType {
    #[serde(rename = "SAFE")]
    Safe,
    #[serde(rename = "SAFE-CREATE")]
    SafeCreate,
}

impl Default for RelayerRequestType {
    fn default() -> Self {
        Self::SafeCreate
    }
}
