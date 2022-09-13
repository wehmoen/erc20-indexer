use std::collections::HashMap;
use thousands::Separable;
use web3::ethabi::{Event, EventParam, ParamType, RawLog};
use web3::types::{BlockId, BlockNumber, Log};
use web3::Web3;
use serde::{Serialize, Deserialize};
use mongodb::{Client};
use mongodb::bson::Document;
use mongodb::options::IndexOptions;
use crate::ContractType::ERC20;

const ERC_TRANSFER_TOPIC: &str =
    "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";

const MONGO_DB_URI: &str = "mongodb://127.0.0.1:27017";
const MONGO_DB_NAME: &str = "ronin-erc20";
const MONGO_DB_COLLECTION_NAME: &str = "transfers";

const MONGO_BATCH_SIZE: usize = 15000;

#[derive(Serialize, Deserialize)]
pub struct Contract {
    pub name: &'static str,
    pub decimals: usize,
    pub erc: ContractType,
    pub address: &'static str,
}

#[derive(Serialize, Deserialize)]
struct Output {
    transfers: Vec<Transfer>
}
pub fn to_string<T: serde::Serialize>(request: &T) -> String {
    web3::helpers::to_string(request).replace('\"', "")
}

#[derive(Serialize, Deserialize, Eq, PartialEq, Hash, Debug, Clone)]
pub enum ContractType {
    ERC20
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Transfer {
    contract: String,
    from: String,
    to: String,
    value: String,
    timestamp: u64
}

pub struct IndexModel {
    pub model: Document,
    pub options: IndexOptions,
}

fn index_model(key: &'static str, unique: bool) -> IndexModel {
    let mut doc = Document::new();
    doc.insert(key, 1u32);

    IndexModel {
        model: doc,
        options: match unique {
            true => IndexOptions::builder().unique(true).build(),
            false => Default::default(),
        },
    }
}

#[tokio::main]
async fn main() {
    let provider = web3::transports::WebSocket::new("ws://127.0.0.1:8546").await.unwrap();
    let web3 =  Web3::new(provider);

    let db_client = Client::with_uri_str(MONGO_DB_URI)
        .await
        .unwrap_or_else(|_| panic!("Failed to connect to mongodb at {}", MONGO_DB_URI));

    let db_indexes: Vec<IndexModel> = vec![
        index_model("contract", false),
        index_model("from", false),
        index_model("to", false),
        index_model("value", false),
        index_model("timestamp", false)
    ];

    let db_db = db_client.database(MONGO_DB_NAME);
    let transfer_collection = db_db.collection::<Transfer>(MONGO_DB_COLLECTION_NAME);

    for model in db_indexes {
            // If indexes exists this will fail silently.
            transfer_collection.create_index(mongodb::IndexModel::builder().keys(model.model).options(model.options).build(), None).await.ok();
    }

    let mut map = HashMap::new();

    #[derive(Serialize, Deserialize)]
    pub struct Contract {
        pub name: &'static str,
        pub decimals: usize,
        pub erc: ContractType,
        pub address: &'static str,
    }

    let contracts_of_interest = [
        "0xc99a6a985ed2cac1ef41640596c5a5f9f4e19ef5",
        "0xed4a9f48a62fb6fdcfb45bb00c9f61d1a436e58c",
        "0xa8754b9fa15fc18bb59458815510e40a12cd2014"
    ];

    map.insert(
        "0xc99a6a985ed2cac1ef41640596c5a5f9f4e19ef5",
        Contract {
            name: "WETH",
            decimals: 18,
            erc: ContractType::ERC20,
            address: "0xc99a6a985ed2cac1ef41640596c5a5f9f4e19ef5",
        },
    );

    map.insert(
        "0xed4a9f48a62fb6fdcfb45bb00c9f61d1a436e58c",
        Contract {
            name: "AXS",
            decimals: 18,
            erc: ContractType::ERC20,
            address: "0xed4a9f48a62fb6fdcfb45bb00c9f61d1a436e58c",
        },
    );

    map.insert(
        "0xa8754b9fa15fc18bb59458815510e40a12cd2014",
        Contract {
            name: "SLP",
            decimals: 0,
            erc: ContractType::ERC20,
            address: "0xa8754b9fa15fc18bb59458815510e40a12cd2014",
        },
    );

    let event = Event {
        name: "Transfer".to_string(),
        inputs: vec![
            EventParam {
                name: "_from".to_string(),
                kind: ParamType::Address,
                indexed: true,
            },
            EventParam {
                name: "_to".to_string(),
                kind: ParamType::Address,
                indexed: true,
            },
            EventParam {
                name: "_value".to_string(),
                kind: ParamType::Uint(256),
                indexed: false,
            },
        ],
        anonymous: false,
    };

    let mut stop = false;
    let mut current_block = 0u64;

    let mut transfer_storage: Vec<Transfer> = vec![];

    let mut total_transfers: u64 = 0;

    loop {

        let chain_head_block = web3
            .eth()
            .block_number()
            .await
            .expect("Failed to retrieve head block number from chain!");

        let stream_stop_block: u64 = chain_head_block.as_u64() - 50;

        let block = web3.eth()
            .block_with_txs(BlockId::Number(BlockNumber::from(current_block as u64)))
            .await
            .unwrap_or_else(|_| panic!("Failed to load block {} from provider!", current_block))
            .unwrap_or_else(|| panic!("Failed to unwrap block {} from result!", current_block));

        let timestamp = block.timestamp.as_u64() * 1000;

        let contracts: Vec<&str> = map
            .values()
            .filter(|c| c.erc == ERC20)
            .map(|c| c.address)
            .collect();

        for tx in block.transactions {
            if let Some(tx_to) = tx.to {
                let tx_to = to_string(&tx_to);
                if contracts_of_interest.contains(&tx_to.as_str()) {
                    let receipt = web3.eth().transaction_receipt(tx.hash).await.unwrap().unwrap();
                    let transfer_log = receipt
                        .logs
                        .iter()
                        .filter(|x| {
                            to_string(&x.topics[0]) == ERC_TRANSFER_TOPIC
                                && contracts.contains(&to_string(&x.address).as_str())
                        })
                        .collect::<Vec<&Log>>();

                    for transfer in transfer_log {
                        let data = event.parse_log(RawLog {
                            topics: transfer.to_owned().topics,
                            data: transfer.to_owned().data.0,
                        }).unwrap();

                        let from = to_string(&data.params[0].value.to_string());
                        let to = to_string(&data.params[1].value.to_string());
                        let value = to_string(&data.params[2].value.to_string());

                        transfer_storage.push(Transfer {
                            contract: tx_to.clone(),
                            from,
                            to,
                            value,
                            timestamp
                        });

                    }
                }
            };
        }

        current_block += 1;

        if current_block > stream_stop_block {
           stop = true
        }

        if transfer_storage.len() >= MONGO_BATCH_SIZE || stop {
            total_transfers += transfer_storage.len()  as u64;
            transfer_collection.insert_many(&transfer_storage, None).await.ok();

            transfer_storage.clear();

        }

        println!("Block: {:>12} Total Transfer: {:>12} Pending Transfer: {:>6}", current_block.separate_with_commas(), total_transfers.separate_with_commas(), transfer_storage.len().separate_with_commas());

        if stop {
            break;
        }
    }
}
