use alloy_network::TransactionBuilder;
use alloy_primitives::{hex, Address};
use alloy_rpc_types::TransactionRequest;
use reqwest::Response;
use serde_json::Value;
use tracing::{debug, info, instrument};

use crate::{
    parse_value, AssembleRequest, AssemblyResponse, ClientConfig, OdosError, OdosHttpClient,
    Result, SwapContext, ASSEMBLE_URL,
};

use super::TransactionData;

use crate::{QuoteRequest, SingleQuoteResponse};

/// The Odos Smart Order Routing V2 API client
#[derive(Debug, Clone)]
pub struct OdosSorV2 {
    client: OdosHttpClient,
}

impl OdosSorV2 {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: OdosHttpClient::new()?,
        })
    }

    pub fn with_config(config: ClientConfig) -> Result<Self> {
        Ok(Self {
            client: OdosHttpClient::with_config(config)?,
        })
    }

    /// Get the client configuration
    pub fn config(&self) -> &ClientConfig {
        self.client.config()
    }

    /// Get a swap quote using Odos API
    ///
    /// Takes a [`QuoteRequest`] and returns a [`SingleQuoteResponse`].
    #[instrument(skip(self), level = "debug")]
    pub async fn get_swap_quote(
        &self,
        quote_request: &QuoteRequest,
    ) -> Result<SingleQuoteResponse> {
        let response = self
            .client
            .execute_with_retry(|| {
                self.client
                    .inner()
                    .post("https://api.odos.xyz/sor/quote/v2")
                    .header("accept", "application/json")
                    .json(quote_request)
            })
            .await?;

        debug!(response = ?response);

        if response.status().is_success() {
            let single_quote_response = response.json().await?;
            Ok(single_quote_response)
        } else {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            Err(OdosError::quote_request_error(format!(
                "API error (status: {status}): {error_text}"
            )))
        }
    }

    #[instrument(skip(self), level = "debug")]
    pub async fn get_assemble_response(
        &self,
        assemble_request: AssembleRequest,
    ) -> Result<Response> {
        self.client
            .execute_with_retry(|| {
                self.client
                    .inner()
                    .post(ASSEMBLE_URL)
                    .header("Content-Type", "application/json")
                    .json(&assemble_request)
            })
            .await
    }

    /// Assemble transaction data from a quote using the Odos Assemble API.
    #[instrument(skip(self), ret(Debug))]
    pub async fn assemble_tx_data(
        &self,
        signer_address: Address,
        output_recipient: Address,
        path_id: &str,
    ) -> Result<TransactionData> {
        let assemble_request = AssembleRequest {
            user_addr: signer_address.to_string(),
            path_id: path_id.to_string(),
            simulate: false,
            receiver: Some(output_recipient),
        };

        let response = self.get_assemble_response(assemble_request).await?;

        if !response.status().is_success() {
            let status = response.status();
            let error = response
                .text()
                .await
                .unwrap_or_else(|_| "Failed to get error message".to_string());

            return Err(OdosError::transaction_assembly_error(format!(
                "API error (status: {status}): {error}"
            )));
        }

        let value: Value = response.json().await?;

        let AssemblyResponse { transaction, .. } = serde_json::from_value(value)?;

        Ok(transaction)
    }

    /// Build a base transaction from a swap using the Odos Assemble API,
    /// leaving gas parameters to be set by the caller.
    #[instrument(skip(self), ret(Debug))]
    pub async fn build_base_transaction(&self, swap: &SwapContext) -> Result<TransactionRequest> {
        let TransactionData { data, value, .. } = self
            .assemble_tx_data(
                swap.signer_address(),
                swap.output_recipient(),
                swap.path_id(),
            )
            .await?;

        info!(value = %value, "Building base transaction");

        Ok(TransactionRequest::default()
            .with_input(hex::decode(&data)?)
            .with_value(parse_value(&value))
            .with_to(swap.router_address())
            .with_from(swap.signer_address()))
    }
}

impl Default for OdosSorV2 {
    fn default() -> Self {
        Self::new().expect("Failed to create default OdosSorV2 client")
    }
}
