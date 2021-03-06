#[macro_use]
extern crate log;

use anyhow::Result;
use rand::Rng;
use rusty_workers::app::AppConfig;
use rusty_workers::kv::KvClient;
use rusty_workers::tarpc;
use rusty_workers::types::*;
use std::net::SocketAddr;
use std::time::SystemTime;
use structopt::StructOpt;
use thiserror::Error;
use tokio::io::AsyncReadExt;

#[derive(Debug, Error)]
enum CliError {
    #[error("bad id128")]
    BadId128,
}

#[derive(Debug, StructOpt)]
#[structopt(name = "rusty-workers-cli", about = "Rusty Workers (cli)")]
struct Opt {
    #[structopt(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, StructOpt)]
enum Cmd {
    /// Connect to runtime.
    Runtime {
        /// Remote address.
        #[structopt(short = "r", long, env = "RUNTIME_ADDR")]
        remote: SocketAddr,

        #[structopt(subcommand)]
        op: RuntimeCmd,
    },

    /// App management.
    App {
        /// TiKV PD address.
        #[structopt(long, env = "TIKV_PD")]
        tikv_pd: String,

        #[structopt(subcommand)]
        op: AppCmd,
    },
}

#[derive(Debug, StructOpt)]
enum AppCmd {
    #[structopt(name = "all-routes")]
    AllRoutes,
    #[structopt(name = "list-routes")]
    ListRoutes { domain: String },
    #[structopt(name = "add-route")]
    AddRoute {
        domain: String,

        #[structopt(long)]
        path: String,

        #[structopt(long)]
        appid: String,
    },
    #[structopt(name = "delete-domain")]
    DeleteDomain { domain: String },
    #[structopt(name = "delete-route")]
    DeleteRoute {
        domain: String,

        #[structopt(long)]
        path: String,
    },
    #[structopt(name = "lookup-route")]
    LookupRoute {
        domain: String,

        #[structopt(long)]
        path: String,
    },
    #[structopt(name = "all-apps")]
    AllApps,
    #[structopt(name = "add-app")]
    AddApp {
        config: String,

        #[structopt(long)]
        bundle: String,
    },
    #[structopt(name = "delete-app")]
    DeleteApp { appid: String },
    #[structopt(name = "get-app")]
    GetApp { appid: String },
    #[structopt(name = "all-bundles")]
    AllBundles,
    #[structopt(name = "delete-bundle")]
    DeleteBundle { id: String },
    #[structopt(name = "list-worker-data")]
    ListWorkerData {
        namespace: String,
        #[structopt(long)]
        from: String,
        #[structopt(long)]
        limit: u32,
        #[structopt(long)]
        base64_key: bool,
    },
    #[structopt(name = "get-worker-data")]
    GetWorkerData {
        namespace: String,
        #[structopt(long)]
        key: String,
        #[structopt(long)]
        base64_key: bool,
        #[structopt(long)]
        base64_value: bool,
    },
    #[structopt(name = "put-worker-data")]
    PutWorkerData {
        namespace: String,
        #[structopt(long)]
        key: String,
        #[structopt(long)]
        value: String,
        #[structopt(long)]
        base64_key: bool,
        #[structopt(long)]
        base64_value: bool,
    },
    #[structopt(name = "delete-worker-data")]
    DeleteWorkerData {
        namespace: String,
        #[structopt(long)]
        key: String,
        #[structopt(long)]
        base64_key: bool,
    },
    #[structopt(name = "delete-worker-data-namespace")]
    DeleteWorkerDataNamespace {
        namespace: String,
        #[structopt(long)]
        batch_size: u32,
    },
    #[structopt(name = "logs")]
    Logs {
        appid: String,
        #[structopt(long)]
        since: String,
    },
    #[structopt(name = "delete-logs")]
    DeleteLogs {
        appid: String,
        #[structopt(long)]
        before: String,
    },
}

#[derive(Debug, StructOpt)]
enum RuntimeCmd {
    #[structopt(name = "spawn")]
    Spawn {
        #[structopt(long, default_value = "")]
        appid: String,

        #[structopt(long)]
        config: Option<String>,

        #[structopt(long)]
        fetch_service: SocketAddr,

        script: String,
    },

    #[structopt(name = "terminate")]
    Terminate { handle: String },

    #[structopt(name = "list")]
    List,

    #[structopt(name = "fetch")]
    Fetch { handle: String },
}

#[tokio::main]
async fn main() -> Result<()> {
    pretty_env_logger::init_timed();
    rusty_workers::init();
    let opt = Opt::from_args();
    match opt.cmd {
        Cmd::Runtime { remote, op } => {
            let mut client = rusty_workers::rpc::RuntimeServiceClient::connect(remote).await?;
            match op {
                RuntimeCmd::Spawn {
                    appid,
                    config,
                    script,
                    fetch_service,
                } => {
                    let config = if let Some(config) = config {
                        let text = read_file(&config).await?;
                        serde_json::from_str(&text)?
                    } else {
                        WorkerConfiguration {
                            executor: ExecutorConfiguration {
                                max_ab_memory_mb: 32,
                                max_time_ms: 50,
                                max_io_concurrency: 10,
                                max_io_per_request: 50,
                            },
                            fetch_service,
                            env: Default::default(),
                            kv_namespaces: Default::default(),
                        }
                    };
                    let script = read_file_raw(&script).await?;
                    let result = client
                        .spawn_worker(make_context(), appid, config, script)
                        .await?;
                    println!("{}", serde_json::to_string(&result).unwrap());
                }
                RuntimeCmd::Terminate { handle } => {
                    let worker_handle = WorkerHandle { id: handle };
                    let result = client
                        .terminate_worker(make_context(), worker_handle)
                        .await?;
                    println!("{}", serde_json::to_string(&result).unwrap());
                }
                RuntimeCmd::List => {
                    let result = client.list_workers(make_context()).await?;
                    println!("{}", serde_json::to_string(&result).unwrap());
                }
                RuntimeCmd::Fetch { handle } => {
                    let worker_handle = WorkerHandle { id: handle };
                    let req = RequestObject::default();
                    let result = client.fetch(make_context(), worker_handle, req).await?;
                    println!("{}", serde_json::to_string(&result).unwrap());
                }
            }
        }
        Cmd::App { tikv_pd, op } => {
            let client = KvClient::new(vec![tikv_pd]).await?;
            match op {
                AppCmd::AllRoutes => {
                    print!("[");
                    let mut first = true;
                    client
                        .route_mapping_for_each(|domain, path, appid| {
                            if first {
                                first = false;
                            } else {
                                print!(",");
                            }
                            print!(
                                "{}",
                                serde_json::to_string(&serde_json::json!({
                                    "domain": domain,
                                    "path": path,
                                    "appid": appid,
                                }))
                                .unwrap()
                            );
                            true
                        })
                        .await?;
                    println!("]");
                }
                AppCmd::ListRoutes { domain } => {
                    let routes = client
                        .route_mapping_list_for_domain(&domain, |_| true)
                        .await?;
                    println!("{}", serde_json::to_string(&routes)?);
                }
                AppCmd::AddRoute {
                    domain,
                    path,
                    appid,
                } => {
                    client.route_mapping_insert(&domain, &path, appid).await?;
                    println!("OK");
                }
                AppCmd::DeleteDomain { domain } => {
                    client.route_mapping_delete_domain(&domain).await?;
                    println!("OK");
                }
                AppCmd::DeleteRoute { domain, path } => {
                    client.route_mapping_delete(&domain, &path).await?;
                    println!("OK");
                }
                AppCmd::LookupRoute { domain, path } => {
                    let result = client.route_mapping_lookup(&domain, &path).await?;
                    println!("{}", serde_json::to_string(&result)?);
                }
                AppCmd::AllApps => {
                    print!("[");
                    let mut first = true;
                    client
                        .app_metadata_for_each(|k| {
                            if first {
                                first = false;
                            } else {
                                print!(",");
                            }
                            print!("{}", serde_json::to_string(k).unwrap());
                            true
                        })
                        .await?;
                    println!("]");
                }
                AppCmd::AddApp { config, bundle } => {
                    let config = read_file(&config).await?;
                    let mut config: AppConfig = toml::from_str(&config)?;
                    let bundle = read_file_raw(&bundle).await?;

                    cleanup_previous_app(&client, &config.id).await?;

                    let mut bundle_id = [0u8; 16];
                    rand::thread_rng().fill(&mut bundle_id);
                    client.app_bundle_put(&bundle_id, bundle).await?;
                    config.bundle_id = rusty_workers::app::encode_id128(&bundle_id);

                    client
                        .app_metadata_put(&config.id.0, serde_json::to_vec(&config)?)
                        .await?;
                    println!("OK");
                }
                AppCmd::DeleteApp { appid } => {
                    let appid = rusty_workers::app::AppId(appid);
                    cleanup_previous_app(&client, &appid).await?;
                    client.app_metadata_delete(&appid.0).await?;

                    client
                        .log_delete_range(
                            &format!("app-{}", appid.0),
                            SystemTime::UNIX_EPOCH..SystemTime::now(),
                        )
                        .await?;

                    println!("OK");
                }
                AppCmd::GetApp { appid } => {
                    let result: Option<AppConfig> = client
                        .app_metadata_get(&appid)
                        .await?
                        .map(|x| serde_json::from_slice(&x))
                        .transpose()?;
                    println!("{}", serde_json::to_string(&result)?);
                }
                AppCmd::AllBundles => {
                    print!("[");
                    let mut first = true;
                    client
                        .app_bundle_for_each(|id| {
                            if first {
                                first = false;
                            } else {
                                print!(",");
                            }
                            print!("{}", serde_json::to_string(&base64::encode(id)).unwrap());
                            true
                        })
                        .await?;
                    println!("]");
                }
                AppCmd::DeleteBundle { id } => {
                    let id = base64::decode(&id)?;
                    client.app_bundle_delete_dirty(&id).await?;
                    println!("OK");
                }
                AppCmd::ListWorkerData {
                    namespace,
                    from,
                    limit,
                    base64_key,
                } => {
                    let namespace = rusty_workers::app::decode_id128(&namespace)
                        .ok_or_else(|| CliError::BadId128)?;
                    let from = if !base64_key {
                        Vec::from(from)
                    } else {
                        base64::decode(&from)?
                    };

                    let keys = client
                        .worker_data_scan_keys(&namespace, &from, None, limit)
                        .await?;
                    let keys: Vec<Option<String>> = keys
                        .into_iter()
                        .map(|k| {
                            if !base64_key {
                                String::from_utf8(k).ok()
                            } else {
                                Some(base64::encode(&k))
                            }
                        })
                        .collect();
                    let serialized = serde_json::to_string(&keys)?;
                    println!("{}", serialized);
                }
                AppCmd::GetWorkerData {
                    namespace,
                    key,
                    base64_key,
                    base64_value,
                } => {
                    let namespace = rusty_workers::app::decode_id128(&namespace)
                        .ok_or_else(|| CliError::BadId128)?;
                    let key = if !base64_key {
                        key.as_bytes().to_vec()
                    } else {
                        base64::decode(&key)?
                    };
                    let value = client.worker_data_get(&namespace, &key).await?;
                    if let Some(v) = value {
                        if !base64_value {
                            println!(
                                "{}",
                                serde_json::to_string(std::str::from_utf8(&v)?).unwrap()
                            );
                        } else {
                            println!("{}", serde_json::to_string(&base64::encode(v)).unwrap());
                        }
                    } else {
                        println!("null");
                    }
                }
                AppCmd::PutWorkerData {
                    namespace,
                    key,
                    value,
                    base64_key,
                    base64_value,
                } => {
                    let namespace = rusty_workers::app::decode_id128(&namespace)
                        .ok_or_else(|| CliError::BadId128)?;
                    let key = if !base64_key {
                        key.as_bytes().to_vec()
                    } else {
                        base64::decode(&key)?
                    };
                    let value = if !base64_value {
                        value.as_bytes().to_vec()
                    } else {
                        base64::decode(&value)?
                    };
                    client.worker_data_put(&namespace, &key, value).await?;
                    println!("OK");
                }
                AppCmd::DeleteWorkerData {
                    namespace,
                    key,
                    base64_key,
                } => {
                    let namespace = rusty_workers::app::decode_id128(&namespace)
                        .ok_or_else(|| CliError::BadId128)?;
                    let key = if !base64_key {
                        key.as_bytes().to_vec()
                    } else {
                        base64::decode(&key)?
                    };
                    client.worker_data_delete(&namespace, &key).await?;
                    println!("OK");
                }
                AppCmd::DeleteWorkerDataNamespace {
                    namespace,
                    batch_size,
                } => {
                    let namespace = rusty_workers::app::decode_id128(&namespace)
                        .ok_or_else(|| CliError::BadId128)?;
                    let keys = client
                        .worker_data_scan_keys(&namespace, b"", None, batch_size)
                        .await?;
                    for k in keys.iter() {
                        client.worker_data_delete(&namespace, k).await?;
                    }
                    println!("{}", keys.len());
                }
                AppCmd::Logs { appid, since } => {
                    let now = SystemTime::now();
                    let since = now - parse_duration::parse(&since)?;

                    print!("[");
                    let mut first = true;
                    client
                        .log_range(&format!("app-{}", appid), since..now, |time, text| {
                            if first {
                                first = false;
                            } else {
                                print!(",");
                            }
                            let item = serde_json::to_string(&serde_json::json!({
                                "time": time,
                                "text": text,
                            }))
                            .unwrap();
                            print!("{}", item);
                            true
                        })
                        .await?;
                    println!("]");
                }
                AppCmd::DeleteLogs { appid, before } => {
                    let end = SystemTime::now() - parse_duration::parse(&before)?;
                    client
                        .log_delete_range(&format!("app-{}", appid), SystemTime::UNIX_EPOCH..end)
                        .await?;
                    println!("OK");
                }
            }
        }
    }
    Ok(())
}

async fn read_file(path: &str) -> Result<String> {
    let mut f = tokio::fs::File::open(path).await?;
    let mut buf = String::new();
    f.read_to_string(&mut buf).await?;
    Ok(buf)
}

async fn read_file_raw(path: &str) -> Result<Vec<u8>> {
    let mut f = tokio::fs::File::open(path).await?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).await?;
    Ok(buf)
}

fn make_context() -> tarpc::context::Context {
    let mut current = tarpc::context::current();
    current.deadline = std::time::SystemTime::now() + std::time::Duration::from_secs(60);
    current
}

async fn cleanup_previous_app(client: &KvClient, appid: &rusty_workers::app::AppId) -> Result<()> {
    if let Some(prev_md) = client.app_metadata_get(&appid.0).await? {
        if let Ok(prev_config) = serde_json::from_slice::<AppConfig>(&prev_md) {
            client
                .app_bundle_delete(
                    &rusty_workers::app::decode_id128(&prev_config.bundle_id)
                        .ok_or_else(|| CliError::BadId128)?,
                )
                .await?;
            warn!("deleted previous bundle");
        } else {
            warn!("unable to decode previous metadata");
        }
    }
    Ok(())
}
