use std::{collections::HashMap, env, net::SocketAddr, path::PathBuf, sync::Arc};

use anyhow::{bail, Context as _, Result};
use bytes::Bytes;
use cid::{
    multihash::{Code, MultihashDigest},
    Cid,
};
use futures::FutureExt;
use iroh::{
    porcelain::provide,
    protocol::GetRequest,
    provider::{create_collection, CustomHandler, DataSource, Database},
    PeerId,
};
use libipld::{cbor::DagCborCodec, prelude::Codec, DagCbor};

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<_> = env::args().collect();
    let direction = &args[1];
    if direction == "client" {
        println!("client");
        let cid = args[2].parse()?;
        let addr = args[3].parse()?;
        let peer = args[4].parse()?;
        let auth_token = args[5].to_string();
        client(cid, addr, peer, auth_token).await?;
    } else if direction == "server" {
        println!("server");
        server().await?;
    } else {
        bail!("unknown '{}' direction", direction);
    }

    Ok(())
}

#[derive(Debug, Clone, DagCbor)]
struct MyCoolData {
    hello: String,
    foo: u32,
    list: Vec<u32>,
}

#[derive(Debug, Clone, DagCbor)]
struct Request {
    action: String,
    cid: Cid,
}

async fn client(cid: Cid, addr: SocketAddr, peer: PeerId, auth_token: String) -> Result<()> {
    println!(
        "requesting {} from {} - {} with {}",
        cid, addr, peer, auth_token
    );

    let opts = iroh::get::Options {
        peer_id: Some(peer),
        addr,
        ..Default::default()
    };
    let token: iroh::protocol::AuthToken = auth_token
        .parse()
        .context("Wrong format for authentication token")?;

    let value = Request {
        action: "get".into(),
        cid,
    };
    let request = DagCborCodec.encode(&value)?;

    tokio::select! {
        biased;
        _ = tokio::signal::ctrl_c() => {
            println!("interupting pull");
        }
        res = iroh::get::run(
            Bytes::from(request).into(),
            token,
            opts,
            || async move { Ok(()) },
            move |mut data| {
                async move {
                    if data.is_root() {
                        let hash = data.request().name;
                        let collection = data.read_collection(hash).await?;
                        data.set_limit(collection.total_entries() + 1);
                        data.user = Some(collection);
                    } else {
                        let index = usize::try_from(data.offset() - 1)?;
                        let hash = data.user.as_ref().unwrap().blobs()[index].hash;
                        let content = data.read_blob(hash).await?;
                        println!("received blob for {hash:?}");
                        tokio::task::spawn_blocking(move || {
                            let res: MyCoolData = DagCborCodec.decode(&content)?;
                            println!("received {:#?}", res);
                            Ok::<_, anyhow::Error>(())
                        }).await??;
                    }
                    data.end()
                }
            },
            None,
        ) => {
            res?;
        }
    };

    Ok(())
}

async fn server() -> Result<()> {
    let tmpdir = tempfile::tempdir()?;

    let db = Database::default();
    let data = MyCoolData {
        hello: "world".into(),
        foo: 4,
        list: vec![1, 3, 8],
    };
    let encoded_data = DagCborCodec.encode(&data)?;
    let cid = Cid::new_v1(DagCborCodec.into(), Code::Blake3_256.digest(&encoded_data));
    let path = tmpdir.path().join(cid.to_string());
    tokio::fs::write(&path, &encoded_data).await?;

    println!("serving {}", cid);
    let custom_handler = Handler {
        data: Arc::new([(cid, path)].into_iter().collect()),
    };
    let provider = provide(db.clone(), None, None, None, false, None, custom_handler).await?;

    iroh::porcelain::display_provider_info(&provider)?;
    let provider2 = provider.clone();
    tokio::select! {
        biased;
        _ = tokio::signal::ctrl_c() => {
            println!("Shutting down provider...");
            provider2.shutdown();
        }
        res = provider => {
            res?;
        }
    }

    Ok(())
}

#[derive(Debug, Clone)]
struct Handler {
    data: Arc<HashMap<Cid, PathBuf>>,
}

impl CustomHandler for Handler {
    fn handle(
        &self,
        data: Bytes,
        database: &Database,
    ) -> futures::future::BoxFuture<'static, Result<GetRequest>> {
        let database = database.clone();
        let this = self.clone();

        async move {
            let req: Request = DagCborCodec.decode(&data)?;
            if req.action == "get" {
                if let Some(path) = this.data.get(&req.cid) {
                    let sources = vec![DataSource::NamedFile {
                        name: "result".into(),
                        path: path.clone(),
                    }];

                    let (new_db, hash) = create_collection(sources).await?;
                    let new_db = new_db.to_inner();
                    database.union_with(new_db);
                    let request = GetRequest::all(hash);
                    println!("{:?}", request);
                    return Ok(request);
                }
            }
            bail!("unknown request: {:#?}", req);
        }
        .boxed()
    }
}
