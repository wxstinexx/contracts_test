// Copyright 2018-2020 Parity Technologies (UK) Ltd.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Operations on the off-chain testing environment.

#[cfg(feature = "ink-unstable-chain-extensions")]
use super::chain_extension::ChainExtension;
pub use super::{
    db::ChainSpec,
    CallData,
    EmittedEvent,
};
use super::{
    db::ExecContext,
    AccountError,
    EnvInstance,
    OnInstance,
};
use crate::{
    Environment,
    Result,
};
use ink_prelude::string::String;

/// Pushes a contract execution context.
///
/// This is the data behind a single instance of a contract call.
///
/// # Note
///
/// Together with [`pop_execution_context`] this can be used to emulated
/// nested calls.
pub fn push_execution_context<T>(
    caller: T::AccountId,
    callee: T::AccountId,
    gas_limit: T::Balance,
    endowment: T::Balance,
    call_data: CallData,
) where
    T: Environment,
{
    <EnvInstance as OnInstance>::on_instance(|instance| {
        instance.exec_context.push(
            ExecContext::build::<T>()
                .caller(caller)
                .callee(callee)
                .gas(gas_limit)
                .transferred_value(endowment)
                .call_data(call_data)
                .finish(),
        )
    })
}

/// Pops the top contract execution context.
///
/// # Note
///
/// Together with [`push_execution_context`] this can be used to emulated
/// nested calls.
pub fn pop_execution_context() {
    <EnvInstance as OnInstance>::on_instance(|instance| {
        instance.exec_context.pop();
    })
}

/// Sets the balance of the account to the given balance.
///
/// # Note
///
/// Note that account could refer to either a user account or
/// a smart contract account.
///
/// # Errors
///
/// - If `account` does not exist.
/// - If the underlying `account` type does not match.
/// - If the underlying `new_balance` type does not match.
pub fn set_account_balance<T>(
    account_id: T::AccountId,
    new_balance: T::Balance,
) -> Result<()>
where
    T: Environment,
{
    <EnvInstance as OnInstance>::on_instance(|instance| {
        instance
            .accounts
            .get_account_mut::<T>(&account_id)
            .ok_or_else(|| AccountError::no_account_for_id::<T>(&account_id))
            .map_err(Into::into)
            .and_then(|account| account.set_balance::<T>(new_balance).map_err(Into::into))
    })
}

/// Returns the balance of the account.
///
/// # Note
///
/// Note that account could refer to either a user account or
/// a smart contract account. This returns the same as `env::api::balance`
/// if given the account ID of the currently executed smart contract.
///
/// # Errors
///
/// - If `account` does not exist.
/// - If the underlying `account` type does not match.
pub fn get_account_balance<T>(account_id: T::AccountId) -> Result<T::Balance>
where
    T: Environment,
{
    <EnvInstance as OnInstance>::on_instance(|instance| {
        instance
            .accounts
            .get_account::<T>(&account_id)
            .ok_or_else(|| AccountError::no_account_for_id::<T>(&account_id))
            .map_err(Into::into)
            .and_then(|account| account.balance::<T>().map_err(Into::into))
    })
}

/// Sets the rent allowance of the contract account to the given rent allowance.
///
/// # Errors
///
/// - If `account` does not exist.
/// - If the underlying `account` type does not match.
/// - If the underlying `new_rent_allowance` type does not match.
pub fn set_contract_rent_allowance<T>(
    account_id: T::AccountId,
    new_rent_allowance: T::Balance,
) -> Result<()>
where
    T: Environment,
{
    <EnvInstance as OnInstance>::on_instance(|instance| {
        instance
            .accounts
            .get_account_mut::<T>(&account_id)
            .ok_or_else(|| AccountError::no_account_for_id::<T>(&account_id))
            .map_err(Into::into)
            .and_then(|account| {
                account
                    .set_rent_allowance::<T>(new_rent_allowance)
                    .map_err(Into::into)
            })
    })
}

/// Returns the rent allowance of the contract account.
///
/// # Errors
///
/// - If `account` does not exist.
/// - If the underlying `account` type does not match.
/// - If the returned rent allowance cannot be properly decoded.
pub fn get_contract_rent_allowance<T>(account_id: T::AccountId) -> Result<T::Balance>
where
    T: Environment,
{
    <EnvInstance as OnInstance>::on_instance(|instance| {
        instance
            .accounts
            .get_account::<T>(&account_id)
            .ok_or_else(|| AccountError::no_account_for_id::<T>(&account_id))
            .map_err(Into::into)
            .and_then(|account| account.rent_allowance::<T>().map_err(Into::into))
    })
}

/// Registers a new chain extension.
#[cfg(feature = "ink-unstable-chain-extensions")]
pub fn register_chain_extension<E, I, O>(extension: E)
where
    E: ChainExtension<Input = I, Output = O> + 'static,
    I: scale::Codec + 'static,
    O: scale::Codec + 'static,
{
    <EnvInstance as OnInstance>::on_instance(|instance| {
        instance
            .chain_extension_handler
            .register(Box::new(extension));
    })
}

/// Set the entropy hash of the current block.
///
/// # Note
///
/// This allows to control what [`crate::random`] returns.
pub fn set_block_entropy<T>(entropy: T::Hash) -> Result<()>
where
    T: Environment,
{
    <EnvInstance as OnInstance>::on_instance(|instance| {
        instance.current_block_mut()?.set_entropy::<T>(entropy)
    })
    .map_err(Into::into)
}

/// Update the [ChainSpec](`crate::test::ChainSpec`) for the test environment
pub fn update_chain_spec<F>(f: F) -> Result<()>
where
    F: FnOnce(&mut ChainSpec),
{
    <EnvInstance as OnInstance>::on_instance(|instance| f(instance.chain_spec_mut()));
    Ok(())
}

/// Returns the contents of the past performed environmental `println` in order.
pub fn recorded_printlns() -> impl Iterator<Item = String> {
    <EnvInstance as OnInstance>::on_instance(|instance| {
        // We return a clone of the recorded strings instead of
        // references to them since this would require the whole `on_instance`
        // API to operate on `'static` environmental instances which would
        // ultimately allow leaking those `'static` references to the outside
        // and potentially lead to terrible bugs such as iterator invalidation.
        instance
            .console
            .past_prints()
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>()
            .into_iter()
    })
}

/// Returns the recorded emitted events in order.
pub fn recorded_events() -> impl Iterator<Item = EmittedEvent> {
    <EnvInstance as OnInstance>::on_instance(|instance| {
        // We return a clone of the recorded emitted events instead of
        // references to them since this would require the whole `on_instance`
        // API to operate on `'static` environmental instances which would
        // ultimately allow leaking those `'static` references to the outside
        // and potentially lead to terrible bugs such as iterator invalidation.
        instance
            .emitted_events
            .emitted_events()
            .map(Clone::clone)
            .collect::<Vec<_>>()
            .into_iter()
    })
}

/// Advances the chain by a single block.
pub fn advance_block<T>() -> Result<()>
where
    T: Environment,
{
    <EnvInstance as OnInstance>::on_instance(|instance| instance.advance_block::<T>())
}

/// The default accounts.
pub struct DefaultAccounts<T>
where
    T: Environment,
{
    /// The predefined `ALICE` account holding substantial amounts of value.
    pub alice: T::AccountId,
    /// The predefined `BOB` account holding some amounts of value.
    pub bob: T::AccountId,
    /// The predefined `CHARLIE` account holding some amounts of value.
    pub charlie: T::AccountId,
    /// The predefined `DJANGO` account holding no value.
    pub django: T::AccountId,
    /// The predefined `EVE` account holding no value.
    pub eve: T::AccountId,
    /// The predefined `FRANK` account holding no value.
    pub frank: T::AccountId,
}

/// Returns the default accounts for testing purposes:
/// Alice, Bob, Charlie, Django, Eve and Frank.
pub fn default_accounts<T>() -> Result<DefaultAccounts<T>>
where
    T: Environment,
    <T as Environment>::AccountId: From<[u8; 32]>,
{
    Ok(DefaultAccounts {
        alice: T::AccountId::from([0x01; 32]),
        bob: T::AccountId::from([0x02; 32]),
        charlie: T::AccountId::from([0x03; 32]),
        django: T::AccountId::from([0x04; 32]),
        eve: T::AccountId::from([0x05; 32]),
        frank: T::AccountId::from([0x06; 32]),
    })
}

/// Initializes the whole off-chain environment.
///
/// # Note
///
/// - Initializes the off-chain environment with default values that fit most
/// uses cases.
pub fn initialize_or_reset_as_default<T>() -> Result<()>
where
    T: Environment,
    <T as Environment>::AccountId: From<[u8; 32]>,
{
    <EnvInstance as OnInstance>::on_instance(|instance| {
        instance.initialize_or_reset_as_default::<T>()
    })
}

/// Runs the given closure test function with the default configuartion
/// for the off-chain environment.
pub fn run_test<T, F>(f: F) -> Result<()>
where
    T: Environment,
    F: FnOnce(DefaultAccounts<T>) -> Result<()>,
    <T as Environment>::AccountId: From<[u8; 32]>,
{
    initialize_or_reset_as_default::<T>()?;
    let default_accounts = default_accounts::<T>()?;
    f(default_accounts)
}

/// Returns the total number of reads and writes of the contract's storage.
pub fn get_contract_storage_rw<T>(account_id: &T::AccountId) -> Result<(usize, usize)>
where
    T: Environment,
{
    <EnvInstance as OnInstance>::on_instance(|instance| {
        instance
            .accounts
            .get_account::<T>(account_id)
            .ok_or_else(|| AccountError::no_account_for_id::<T>(account_id))
            .map_err(Into::into)
            .and_then(|account| account.get_storage_rw().map_err(Into::into))
    })
}
