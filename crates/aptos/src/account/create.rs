// Copyright (c) Aptos
// SPDX-License-Identifier: Apache-2.0

//! A command to create a new account on-chain
//!
//! TODO: Examples
//!

use crate::{
    common::types::{EncodingOptions, NodeOptions, PrivateKeyInputOptions},
    CliResult, Error as CommonError,
};
use anyhow::Error;
use aptos_crypto::{
    ed25519::{Ed25519PrivateKey, Ed25519PublicKey},
    PrivateKey,
};
use aptos_rest_client::{Client as RestClient, Response, Transaction};
use aptos_sdk::{
    transaction_builder::TransactionFactory,
    types::{chain_id::ChainId, transaction::authenticator::AuthenticationKey, LocalAccount},
};
use aptos_transaction_builder::aptos_stdlib;
use aptos_types::account_address::AccountAddress;
use clap::Parser;
use reqwest;

/// Command to create a new account on-chain
///
#[derive(Debug, Parser)]
pub struct CreateAccount {
    #[clap(flatten)]
    private_key_input_options: PrivateKeyInputOptions,

    #[clap(flatten)]
    encoding_options: EncodingOptions,

    #[clap(flatten)]
    node: NodeOptions,

    /// Public Key of account you want to create
    public_key: String,

    /// Chain ID
    chain_id: u8,

    /// Flag for using faucet
    #[clap(long)]
    use_faucet: bool,
}

impl CreateAccount {
    async fn get_account(
        &self,
        account: AccountAddress,
    ) -> Result<serde_json::Value, reqwest::Error> {
        reqwest::get(format!("{}accounts/{}", self.node.url, account))
            .await?
            .json()
            .await
    }

    fn get_address(&self) -> Result<AccountAddress, String> {
        let public_key: Ed25519PublicKey = self
            .encoding_options
            .encoding
            .decode_key(self.public_key.as_bytes().to_vec())
            .map_err(|err| err.to_string())?;
        let auth_key = AuthenticationKey::ed25519(&public_key);
        Ok(AccountAddress::new(*auth_key.derived_address()))
    }

    async fn get_sequence_number(&self, account: AccountAddress) -> Result<u64, CommonError> {
        let account_response = self
            .get_account(account)
            .await
            .map_err(|err| CommonError::UnexpectedError(err.to_string()))?;
        let sequence_number = &account_response["sequence_number"];
        match sequence_number.as_str() {
            Some(number) => Ok(number.parse::<u64>().unwrap()),
            None => Err(CommonError::UnexpectedError(
                "Sequence number not found".to_string(),
            )),
        }
    }

    async fn post_account(
        &self,
        address: AccountAddress,
        sender_key: Ed25519PrivateKey,
        sender_address: AccountAddress,
        sequence_number: u64,
    ) -> Result<Response<Transaction>, Error> {
        let client = RestClient::new(reqwest::Url::clone(&self.node.url));
        let chain_id = ChainId::new(self.chain_id);
        let transaction_factory = TransactionFactory::new(chain_id)
            .with_gas_unit_price(1)
            .with_max_gas_amount(1000);
        let sender_account = &mut LocalAccount::new(sender_address, sender_key, sequence_number);
        let transaction = sender_account.sign_with_transaction_builder(
            transaction_factory
                .payload(aptos_stdlib::encode_create_account_script_function(address)),
        );
        client.submit_and_wait(&transaction).await
    }

    async fn create_account_with_faucet(self, address: AccountAddress) -> Result<String, Error> {
        let response = reqwest::Client::new()
            // TODO: Currently, we are just using mint 0 to create an account using the faucet
            // We should make a faucet endpoint for creating an account
            .post(format!(
                "{}/mint?amount={}&auth_key={}",
                "https://faucet.devnet.aptoslabs.com", "0", address
            ))
            .send()
            .await?;
        if response.status() == 200 {
            Ok(response.status().to_string())
        } else {
            Err(Error::new(CommonError::UnexpectedError(format!(
                "Faucet issue: {}",
                response.status()
            ))))
        }
    }

    async fn create_account_with_key(self, address: AccountAddress) -> Result<String, Error> {
        let private_key = self
            .private_key_input_options
            .extract_private_key(self.encoding_options.encoding)?
            .ok_or(CommonError::UnexpectedError(
                "One of ['--private-key', '--private-key-file', '--use-faucet'] must be provided"
                    .to_string(),
            ))?;
        let sender_address =
            AuthenticationKey::ed25519(&private_key.public_key()).derived_address();
        let sender_address = AccountAddress::new(*sender_address);
        let sequence_number = self.get_sequence_number(sender_address).await;
        match sequence_number {
            Ok(sequence_number) => self
                .post_account(address, private_key, sender_address, sequence_number)
                .await
                .map(|_| "Success".to_string()),
            Err(err) => Err(Error::new(err)),
        }
    }

    async fn execute_inner(self, address: AccountAddress) -> Result<String, Error> {
        if self.use_faucet {
            self.create_account_with_faucet(address).await
        } else {
            self.create_account_with_key(address).await
        }
    }

    pub async fn execute(self) -> CliResult {
        let address = self.get_address()?;
        self.execute_inner(address)
            .await
            .map(|_| format!("Account Created at {}", address))
            .map_err(|err| err.to_string())
    }
}
