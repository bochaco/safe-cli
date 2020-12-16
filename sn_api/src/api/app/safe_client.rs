// Copyright 2020 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under the MIT license <LICENSE-MIT
// http://opensource.org/licenses/MIT> or the Modified BSD license <LICENSE-BSD
// https://opensource.org/licenses/BSD-3-Clause>, at your option. This file may not be copied,
// modified, or distributed except according to those terms. Please review the Licences for the
// specific language governing permissions and limitations relating to use of the SAFE Network
// Software.

use super::{fetch::Range, helpers::xorname_to_hex};
use crate::{
    api::ipc::{IpcMsg, IpcResp},
    Error, Result,
};

use log::{debug, info};
use sn_client::{Client, ClientError as SafeClientError};
use sn_data_types::{
    Blob, BlobAddress, Error as SafeNdError, Keypair, Map, MapAction, MapAddress, MapEntryActions,
    MapPermissionSet, MapSeqEntryActions, MapSeqValue, MapValue, Money, PublicBlob, PublicKey,
    SequenceAddress, SequenceIndex, SequencePrivatePermissions, SequencePublicPermissions,
    SequenceUser,
};

use std::collections::BTreeMap;
use std::sync::Arc;
use std::{collections::HashSet, net::SocketAddr};
use xor_name::XorName;

const APP_NOT_CONNECTED: &str = "Application is not connected to the network";

#[derive(Default, Clone)]
pub struct SafeAppClient {
    safe_client: Option<Client>,
    pub(crate) bootstrap_config: Option<HashSet<SocketAddr>>,
}

impl SafeAppClient {
    // Private helper to obtain the Safe Client instance
    fn get_safe_client(&self) -> Result<Client> {
        match &self.safe_client {
            Some(client) => Ok(client.clone()),
            None => Err(Error::ConnectionError(APP_NOT_CONNECTED.to_string())),
        }
    }

    pub fn new() -> Self {
        Self {
            safe_client: None,
            bootstrap_config: None,
        }
    }

    pub async fn keypair(&self) -> Result<Arc<Keypair>> {
        let client = self.get_safe_client()?;
        let kp = client.keypair().await;

        Ok(kp)
    }

    // Connect to the SAFE Network using the provided auth credentials
    pub async fn connect(&mut self, auth_credentials: Option<&str>) -> Result<()> {
        debug!("Connecting to SAFE Network...");

        let app_keypair = if let Some(auth_credentials) = auth_credentials {
            match IpcMsg::from_string(auth_credentials)? {
                IpcMsg::Resp(IpcResp::Auth(Ok(auth_granted))) => {
                    self.bootstrap_config = Some(auth_granted.bootstrap_config);
                    debug!(
                        "Client instantiated with pk: {:?}",
                        auth_granted.app_keypair.public_key()
                    );
                    Some(auth_granted.app_keypair)
                }
                IpcMsg::Resp(IpcResp::Unregistered(Ok(bootstrap_config))) => {
                    // unregistered type used for returning bootstrap config for client
                    self.bootstrap_config = Some(bootstrap_config);
                    None
                }
                IpcMsg::Resp(IpcResp::Auth(Err(e)))
                | IpcMsg::Resp(IpcResp::Unregistered(Err(e)))
                | IpcMsg::Err(e) => return Err(Error::AuthError(format!("{:?}", e))),
                IpcMsg::Req(req) => {
                    return Err(Error::AuthError(format!("Invalid credentials: {:?}", req)))
                }
            }
        } else {
            None
        };

        let client = Client::new(app_keypair, self.bootstrap_config.clone())
            .await
            .map_err(|err| {
                Error::ConnectionError(format!("Failed to connect to the SAFE Network: {:?}", err))
            })?;

        self.safe_client = Some(client);

        debug!("Successfully connected to the Network!!!");
        Ok(())
    }

    // === Money operations ===
    pub async fn read_balance_from_keypair(&self, id: Arc<Keypair>) -> Result<Money> {
        let temp_client = Client::new(Some(id), self.bootstrap_config.clone()).await?;
        temp_client.get_balance().await.map_err(|err| match err {
            SafeClientError::DataError(SafeNdError::NoSuchBalance) => {
                Error::ContentNotFound("No SafeKey found at specified location".to_string())
            }
            other => Error::NetDataError(format!("Failed to retrieve balance: {:?}", other)),
        })
    }

    #[cfg(feature = "simulated-payouts")]
    pub async fn trigger_simulated_farming_payout(
        &mut self,
        amount: Money,
        id: Option<Arc<Keypair>>,
    ) -> Result<()> {
        let mut client = if id.is_some() {
            Client::new(id, self.bootstrap_config.clone()).await?
        } else {
            self.get_safe_client()?
        };

        client.trigger_simulated_farming_payout(amount).await?;

        Ok(())
    }

    pub async fn safecoin_transfer_to_xorname(
        &self,
        from_id: Option<Arc<Keypair>>,
        to_xorname: XorName,
        amount: Money,
    ) -> Result<u64> {
        // Get pk from xorname. We assume Ed25519 key for now, which is
        // 32 bytes long, just like a xorname.
        // TODO: support for BLS keys which are longer.
        let to_pk = ed25519_dalek::PublicKey::from_bytes(to_xorname.as_ref()).map_err(|err| {
            Error::NetDataError(format!(
                "Failed to derive Ed25519 PublicKey from Xorname '{}': {:?}",
                to_xorname, err
            ))
        })?;

        self.safecoin_transfer_to_pk(from_id, to_pk.into(), amount)
            .await
    }

    pub async fn safecoin_transfer_to_pk(
        &self,
        from_id: Option<Arc<Keypair>>,
        to_pk: PublicKey,
        amount: Money,
    ) -> Result<u64> {
        let client = match from_id {
            Some(id) => Client::new(Some(id), self.bootstrap_config.clone()).await?,
            None => self.get_safe_client()?,
        };

        let (dot_counter, _dot_actor) =
            client
                .send_money(to_pk, amount)
                .await
                .map_err(|err| match err {
                    SafeClientError::DataError(SafeNdError::InsufficientBalance) => {
                        Error::NotEnoughBalance(format!(
                            "Not enough balance at 'source' for the operation: {}",
                            amount
                        ))
                    }
                    SafeClientError::DataError(SafeNdError::ExcessiveValue)
                    | SafeClientError::DataError(SafeNdError::InvalidOperation) => {
                        Error::InvalidAmount(format!(
                            "The amount '{}' specified for the transfer is invalid",
                            amount
                        ))
                    }
                    other => Error::NetDataError(format!("Failed to transfer coins: {:?}", other)),
                })?;

        // TODO: perhaps include the actor as part of the TX ID
        Ok(dot_counter)
    }

    // // === Blob operations ===
    pub async fn store_public_blob(&self, data: &[u8], dry_run: bool) -> Result<XorName> {
        // TODO: allow this operation to work without a connection when it's a dry run
        let client = self.get_safe_client()?;

        let data_vec = data.to_vec();
        let blob_for_storage = Blob::Public(PublicBlob::new(data_vec));
        let xorname = *blob_for_storage.address().name();

        if !dry_run {
            client
                .store_public_blob(data)
                .await
                .map_err(|e| Error::NetDataError(format!("Failed to PUT Public Blob: {:?}", e)))?;
        }

        Ok(xorname)
    }

    pub async fn get_public_blob(&self, xorname: XorName, range: Range) -> Result<Vec<u8>> {
        debug!("Fetching immutable data: {:?}", &xorname);

        let client = self.get_safe_client()?;
        let blob_address = BlobAddress::Public(xorname);
        let data = if let Some((start, end)) = range {
            let len = if let Some(end_index) = end {
                Some(end_index - start.unwrap_or(0))
            } else {
                None
            };
            client.read_blob(blob_address, start, len).await
        } else {
            client.read_blob(blob_address, None, None).await
        }
        .map_err(|e| Error::NetDataError(format!("Failed to GET Public Blob: {:?}", e)))?;

        debug!(
            "Public Blob data successfully retrieved from: {:?}",
            &xorname
        );

        Ok(data)
    }

    // === Map operations ===
    pub async fn store_map(
        &self,
        name: Option<XorName>,
        tag: u64,
        _data: Option<String>,
        _permissions: Option<String>,
    ) -> Result<XorName> {
        let xorname = name.unwrap_or_else(rand::random);

        // The Map's owner will be the client's public key
        let client = self.get_safe_client()?;
        let owner = client.public_key().await;

        let permission_set = MapPermissionSet::new()
            .allow(MapAction::Read)
            .allow(MapAction::Insert)
            .allow(MapAction::Update)
            .allow(MapAction::Delete)
            .allow(MapAction::ManagePermissions);

        let mut permission_map = BTreeMap::new();
        let app_pk = client.public_key().await;
        permission_map.insert(app_pk, permission_set);

        client
            .store_seq_map(
                xorname,
                tag,
                owner,
                Some(BTreeMap::new()),
                Some(permission_map),
            )
            .await
            .map_err(|err| Error::NetDataError(format!("Failed to store SeqMap: {}", err)))?;

        Ok(xorname)
    }

    #[allow(dead_code)]
    pub async fn get_map(&self, name: XorName, tag: u64) -> Result<Map> {
        let client = self.get_safe_client()?;
        let address = MapAddress::Seq { name, tag };

        client
            .get_map(address)
            .await
            .map_err(|e| Error::NetDataError(format!("Failed to get SeqMap: {:?}", e)))
    }

    pub async fn map_insert(
        &self,
        name: XorName,
        tag: u64,
        key: &[u8],
        value: &[u8],
    ) -> Result<()> {
        let entry_actions = MapSeqEntryActions::new();
        let entry_actions = entry_actions.ins(key.to_vec(), value.to_vec(), 0);
        self.edit_map_entries(name, tag, entry_actions, "Failed to insert to SeqMap")
            .await
    }

    pub async fn map_get_value(&self, name: XorName, tag: u64, key: &[u8]) -> Result<MapValue> {
        let client = self.get_safe_client()?;
        let key_vec = key.to_vec();
        let address = MapAddress::Seq { name, tag };

        client
            .get_map_value(address, key_vec)
            .await
            .map_err(|err| match err {
                SafeClientError::DataError(SafeNdError::AccessDenied) => {
                    Error::AccessDenied(format!("Failed to retrieve a key: {:?}", key))
                }
                SafeClientError::DataError(SafeNdError::NoSuchData) => {
                    Error::ContentNotFound(format!(
                        "Sequenced Map not found at Xor name: {}",
                        xorname_to_hex(&name)
                    ))
                }
                SafeClientError::DataError(SafeNdError::NoSuchEntry) => {
                    Error::EntryNotFound(format!(
                        "Entry not found in Sequenced Map found at Xor name: {}",
                        xorname_to_hex(&name)
                    ))
                }
                err => Error::NetDataError(format!("Failed to retrieve a key. {:?}", err)),
            })
    }

    pub async fn list_map_entries(
        &self,
        name: XorName,
        tag: u64,
    ) -> Result<BTreeMap<Vec<u8>, MapSeqValue>> {
        let client = self.get_safe_client()?;
        client
            .list_seq_map_entries(name, tag)
            .await
            .map_err(|err| match err {
                SafeClientError::DataError(SafeNdError::AccessDenied) => {
                    Error::AccessDenied(format!(
                        "Failed to get Sequenced Map at: {:?} (type tag: {})",
                        name, tag
                    ))
                }
                SafeClientError::DataError(SafeNdError::NoSuchData) => {
                    Error::ContentNotFound(format!(
                        "Sequenced Map not found at Xor name: {} (type tag: {})",
                        xorname_to_hex(&name),
                        tag
                    ))
                }
                SafeClientError::DataError(SafeNdError::NoSuchEntry) => {
                    Error::EntryNotFound(format!(
                        "Entry not found in Sequenced Map found at Xor name: {} (type tag: {})",
                        xorname_to_hex(&name),
                        tag
                    ))
                }
                err => Error::NetDataError(format!("Failed to get Sequenced Map. {:?}", err)),
            })
    }

    async fn edit_map_entries(
        &self,
        name: XorName,
        tag: u64,
        entry_actions: MapSeqEntryActions,
        error_msg: &str,
    ) -> Result<()> {
        let client = self.get_safe_client()?;
        let message = error_msg.to_string();
        let address = MapAddress::Seq { name, tag };
        client
            .edit_map_entries(address, MapEntryActions::Seq(entry_actions))
            .await
            .map_err(|err| {
                if let SafeClientError::DataError(SafeNdError::InvalidEntryActions(_)) = err {
                    Error::EntryExists(format!("{}: {}", message, err))
                } else {
                    Error::NetDataError(format!("{}: {}", message, err))
                }
            })
    }

    pub async fn update_map(
        &self,
        name: XorName,
        tag: u64,
        key: &[u8],
        value: &[u8],
        version: u64,
    ) -> Result<()> {
        let entry_actions = MapSeqEntryActions::new();
        let entry_actions = entry_actions.update(key.to_vec(), value.to_vec(), version);
        self.edit_map_entries(name, tag, entry_actions, "Failed to update SeqMD")
            .await
    }

    // === Sequence data operations ===
    pub async fn store_sequence(
        &self,
        data: &[u8],
        name: Option<XorName>,
        tag: u64,
        _permissions: Option<String>,
        private: bool,
    ) -> Result<XorName> {
        debug!(
            "Storing {} Sequence data with tag type: {:?}, xorname: {:?}",
            if private { "Private" } else { "Public" },
            tag,
            name
        );

        let client = self.get_safe_client()?;
        let xorname = name.unwrap_or_else(rand::random);
        info!("Xorname for storage: {:?}", &xorname);

        // The Sequence's owner will be the client's public key
        let owner = client.public_key().await;

        // Store the Sequence on the network
        let _address = if private {
            // Set permissions for append, delete, and manage perms to this application
            let mut perms = BTreeMap::default();
            let _ = perms.insert(owner, SequencePrivatePermissions::new(true, true, true));

            client
                .store_private_sequence(Some(vec![data.to_vec()]), xorname, tag, owner, perms)
                .await
                .map_err(|e| {
                    Error::NetDataError(format!("Failed to store Private Sequence data: {:?}", e))
                })?
        } else {
            // Set permissions for append and manage perms to this application
            let user_app = SequenceUser::Key(owner);
            let mut perms = BTreeMap::default();
            let _ = perms.insert(user_app, SequencePublicPermissions::new(true, true));

            client
                .store_public_sequence(Some(vec![data.to_vec()]), xorname, tag, owner, perms)
                .await
                .map_err(|e| {
                    Error::NetDataError(format!("Failed to store Public Sequence data: {:?}", e))
                })?
        };

        Ok(xorname)
    }

    pub async fn sequence_get_last_entry(
        &self,
        name: XorName,
        tag: u64,
        private: bool,
    ) -> Result<(u64, Vec<u8>)> {
        debug!(
            "Fetching {} Sequence data w/ type: {:?}, xorname: {:?}",
            if private { "Private" } else { "Public" },
            tag,
            name
        );

        let client = self.get_safe_client()?;

        let sequence_address = if private {
            SequenceAddress::Private { name, tag }
        } else {
            SequenceAddress::Public { name, tag }
        };

        client
            .get_sequence_last_entry(sequence_address)
            .await
            .map_err(|err| {
                if let SafeClientError::DataError(SafeNdError::NoSuchEntry) = err {
                    Error::EmptyContent(format!("Empty Sequence found at XoR name {}", name))
                } else {
                    Error::NetDataError(format!(
                        "Failed to retrieve last entry from Sequence data: {:?}",
                        err
                    ))
                }
            })
    }

    pub async fn sequence_get_entry(
        &self,
        name: XorName,
        tag: u64,
        index: u64,
        private: bool,
    ) -> Result<Vec<u8>> {
        debug!(
            "Fetching {} Sequence data w/ type: {:?}, xorname: {:?}",
            if private { "Private" } else { "Public" },
            tag,
            name
        );

        let client = self.get_safe_client()?;

        let sequence_address = if private {
            SequenceAddress::Private { name, tag }
        } else {
            SequenceAddress::Public { name, tag }
        };
        let start = SequenceIndex::FromStart(index);
        let end = SequenceIndex::FromStart(index + 1);

        let res = client
            .get_sequence_range(sequence_address, (start, end))
            .await
            .map_err(|err| {
                if let SafeClientError::DataError(SafeNdError::NoSuchEntry) = err {
                    Error::VersionNotFound(format!(
                        "Invalid version ({}) for Sequence found at XoR name {}",
                        index, name
                    ))
                } else {
                    Error::NetDataError(format!(
                        "Failed to retrieve entry at index {} from Sequence data: {:?}",
                        index, err
                    ))
                }
            })?;

        let entry = res.get(0).ok_or_else(|| {
            Error::EmptyContent(format!(
                "Empty Sequence found at Xor name {}",
                xorname_to_hex(&name)
            ))
        })?;

        Ok(entry.to_vec())
    }

    pub async fn append_to_sequence(
        &self,
        data: &[u8],
        name: XorName,
        tag: u64,
        private: bool,
    ) -> Result<()> {
        debug!(
            "Appending to {} Sequence data w/ type: {:?}, xorname: {:?}",
            if private { "Private" } else { "Public" },
            tag,
            name
        );

        let client = self.get_safe_client()?;

        let sequence_address = if private {
            SequenceAddress::Private { name, tag }
        } else {
            SequenceAddress::Public { name, tag }
        };

        client
            .append_to_sequence(sequence_address, data.to_vec())
            .await
            .map_err(|e| Error::NetDataError(format!("Failed to append to Sequence: {:?}", e)))
    }
}
