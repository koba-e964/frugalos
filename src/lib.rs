//! Frugal Object Storage.
#![warn(missing_docs)]
#![allow(clippy::new_ret_no_self)]
extern crate atomic_immut;
extern crate bytecodec;
extern crate cannyls;
extern crate cannyls_rpc;
extern crate fibers;
extern crate fibers_http_server;
extern crate fibers_rpc;
extern crate fibers_tasque;
extern crate frugalos_config;
extern crate frugalos_core;
extern crate frugalos_mds;
extern crate frugalos_raft;
extern crate frugalos_segment;
extern crate futures;
extern crate httpcodec;
extern crate jemalloc_ctl;
extern crate libfrugalos;
extern crate num_cpus;
extern crate prometrics;
extern crate raftlog;
extern crate rustracing;
extern crate rustracing_jaeger;
extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate serde_yaml;
extern crate siphasher;
extern crate url;
#[macro_use]
extern crate slog;
#[cfg(test)]
extern crate tempdir;
#[macro_use]
extern crate trackable;

use std::fs::File;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

macro_rules! dump {
    ($($e:expr),*) => {
        format!(concat!($(stringify!($e), "={:?}; "),*), $($e),*)
    }
}

pub use error::{Error, ErrorKind};

pub mod daemon;

mod bucket;
mod client;
mod codec;
mod config_server;
mod error;
mod http;
mod rpc_server;
mod server;
mod service;

/// クレート固有の`Result`型。
pub type Result<T> = ::std::result::Result<T, Error>;

/// ファイルに書き出した時のフォーマットを調整する。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct FrugalosConfigWrapper {
    #[serde(rename = "frugalos")]
    #[serde(default)]
    config: FrugalosConfig,
}

/// frugalos の設定を表す struct。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrugalosConfig {
    /// データ用ディレクトリのパス。
    #[serde(default)]
    pub data_dir: String,
    /// ログをファイルに出力する場合の出力先ファイルパス。
    #[serde(default)]
    pub log_file: Option<PathBuf>,
    /// 出力するログレベルの下限。
    #[serde(default = "default_loglevel")]
    pub loglevel: sloggers::types::Severity,
    /// 同時に処理できるログの最大値。
    #[serde(default = "default_max_concurrent_logs")]
    pub max_concurrent_logs: usize,
    /// デーモン向けの設定。
    #[serde(default)]
    pub daemon: FrugalosDaemonConfig,
    /// HTTP server 向けの設定。
    #[serde(default)]
    pub http_server: FrugalosHttpServerConfig,
    /// RPC server 向けの設定。
    #[serde(default)]
    pub rpc_server: FrugalosRpcServerConfig,
    /// frugalos_mds 向けの設定。
    #[serde(default)]
    pub mds: frugalos_mds::FrugalosMdsConfig,
    /// frugalos_segment 向けの設定。
    #[serde(default)]
    pub segment: frugalos_segment::FrugalosSegmentConfig,
}

impl FrugalosConfig {
    /// Reads `FrugalosConfig` from a YAML file.
    pub fn from_yaml<P: AsRef<Path>>(path: P) -> Result<FrugalosConfig> {
        let file = File::open(path.as_ref()).map_err(|e| track!(Error::from(e)))?;
        serde_yaml::from_reader::<File, FrugalosConfigWrapper>(file)
            .map(|wrapped| wrapped.config)
            .map_err(|e| track!(Error::from(e)))
    }
}

impl Default for FrugalosConfig {
    fn default() -> Self {
        Self {
            data_dir: Default::default(),
            log_file: Default::default(),
            loglevel: default_loglevel(),
            max_concurrent_logs: default_max_concurrent_logs(),
            daemon: Default::default(),
            http_server: Default::default(),
            rpc_server: Default::default(),
            mds: Default::default(),
            segment: Default::default(),
        }
    }
}

/// `FrugalosDaemon` 向けの設定。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrugalosDaemonConfig {
    /// 実行スレッド数。
    #[serde(default = "default_executor_threads")]
    pub executor_threads: usize,

    /// Jaegerのトレースのサンプリング確率。
    #[serde(default = "default_sampling_rate")]
    pub sampling_rate: f64,

    /// frugalos 停止時に待つ時間。
    #[serde(
        rename = "stop_waiting_time_millis",
        default = "default_stop_waiting_time",
        with = "frugalos_core::serde_ext::duration_millis"
    )]
    pub stop_waiting_time: Duration,
}

impl Default for FrugalosDaemonConfig {
    fn default() -> FrugalosDaemonConfig {
        Self {
            executor_threads: default_executor_threads(),
            sampling_rate: default_sampling_rate(),
            stop_waiting_time: default_stop_waiting_time(),
        }
    }
}

/// HTTP server 向けの設定。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrugalosHttpServerConfig {
    /// bind するアドレス。
    #[serde(default = "default_http_server_bind_addr")]
    pub bind_addr: SocketAddr,
}

impl Default for FrugalosHttpServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: default_http_server_bind_addr(),
        }
    }
}

/// RPC server 向けの設定。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrugalosRpcServerConfig {
    /// bind するアドレス。
    #[serde(default = "default_rpc_server_bind_addr")]
    pub bind_addr: SocketAddr,

    /// RPC の接続タイムアウト時間。
    #[serde(
        rename = "tcp_connect_timeout_millis",
        default = "default_tcp_connect_timeout",
        with = "frugalos_core::serde_ext::duration_millis"
    )]
    pub tcp_connect_timeout: Duration,

    /// RPC の書き込みタイムアウト時間。
    #[serde(
        rename = "tcp_write_timeout_millis",
        default = "default_tcp_write_timeout",
        with = "frugalos_core::serde_ext::duration_millis"
    )]
    pub tcp_write_timeout: Duration,
}

impl FrugalosRpcServerConfig {
    fn channel_options(&self) -> fibers_rpc::channel::ChannelOptions {
        let mut options = fibers_rpc::channel::ChannelOptions::default();
        options.tcp_connect_timeout = self.tcp_connect_timeout;
        options.tcp_write_timeout = self.tcp_write_timeout;
        options
    }
}

impl Default for FrugalosRpcServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: default_rpc_server_bind_addr(),
            tcp_connect_timeout: default_tcp_connect_timeout(),
            tcp_write_timeout: default_tcp_write_timeout(),
        }
    }
}

fn default_executor_threads() -> usize {
    num_cpus::get()
}

fn default_sampling_rate() -> f64 {
    0.001
}

fn default_stop_waiting_time() -> Duration {
    Duration::from_millis(10)
}

fn default_http_server_bind_addr() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 3000))
}

fn default_rpc_server_bind_addr() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 14278))
}

fn default_tcp_connect_timeout() -> Duration {
    Duration::from_secs(5)
}

fn default_tcp_write_timeout() -> Duration {
    Duration::from_secs(5)
}

fn default_loglevel() -> sloggers::types::Severity {
    sloggers::types::Severity::Info
}

fn default_max_concurrent_logs() -> usize {
    4096
}

#[cfg(test)]
mod tests {
    use super::*;
    use libfrugalos::time::Seconds;
    use std::fs::File;
    use std::io::Write;
    use tempdir::TempDir;
    use trackable::result::TestResult;

    #[test]
    fn config_works() -> TestResult {
        let content = r##"
---
frugalos:
  data_dir: "/var/lib/frugalos"
  log_file: ~
  loglevel: critical
  max_concurrent_logs: 30
  daemon:
    executor_threads: 3
    sampling_rate: 0.1
    stop_waiting_time_millis: 300
  http_server:
    bind_addr: "127.0.0.1:2222"
  rpc_server:
    bind_addr: "127.0.0.1:3333"
    tcp_connect_timeout_millis: 8000
    tcp_write_timeout_millis: 10000
  mds:
    commit_timeout_threshold: 20
    large_proposal_queue_threshold: 250
    large_leader_waiting_queue_threshold: 400
    leader_waiting_timeout_threshold: 12
    node_polling_interval_millis: 200
    reelection_threshold: 48
    snapshot_threshold_min: 100
    snapshot_threshold_max: 200
  segment:
    dispersed_client:
      get_timeout_millis: 4000
    mds_client:
      put_content_timeout_secs: 32"##;
        let dir = track_any_err!(TempDir::new("frugalos_test"))?;
        let filepath = dir.path().join("frugalos1.yml");
        let mut file = track_any_err!(File::create(filepath.clone()))?;

        track_any_err!(file.write(content.as_bytes()))?;

        let actual = track!(FrugalosConfig::from_yaml(filepath))?;
        let mut expected = FrugalosConfig::default();
        expected.data_dir = "/var/lib/frugalos".to_owned();
        expected.max_concurrent_logs = 30;
        expected.loglevel = sloggers::types::Severity::Critical;
        expected.daemon.sampling_rate = 0.1;
        expected.daemon.executor_threads = 3;
        expected.daemon.stop_waiting_time = Duration::from_millis(300);
        expected.http_server.bind_addr = SocketAddr::from(([127, 0, 0, 1], 2222));
        expected.rpc_server.bind_addr = SocketAddr::from(([127, 0, 0, 1], 3333));
        expected.rpc_server.tcp_connect_timeout = Duration::from_secs(8);
        expected.rpc_server.tcp_write_timeout = Duration::from_secs(10);
        expected.mds.commit_timeout_threshold = 20;
        expected.mds.large_proposal_queue_threshold = 250;
        expected.mds.large_leader_waiting_queue_threshold = 400;
        expected.mds.leader_waiting_timeout_threshold = 12;
        expected.mds.node_polling_interval = Duration::from_millis(200);
        expected.mds.reelection_threshold = 48;
        expected.mds.snapshot_threshold_min = 100;
        expected.mds.snapshot_threshold_max = 200;
        expected.segment.dispersed_client.get_timeout = Duration::from_secs(4);
        expected.segment.mds_client.put_content_timeout = Seconds(32);

        assert_eq!(expected, actual);

        Ok(())
    }

    #[test]
    fn default_config_values_is_used() -> TestResult {
        let content = r##"---
        frugalos: {}
        "##;
        let dir = track_any_err!(TempDir::new("frugalos_test"))?;
        let filepath = dir.path().join("frugalos2.yml");
        let mut file = track_any_err!(File::create(filepath.clone()))?;

        track_any_err!(file.write(content.as_bytes()))?;

        let actual = track!(FrugalosConfig::from_yaml(filepath))?;
        assert_eq!(FrugalosConfig::default(), actual);

        Ok(())
    }

    #[test]
    fn it_works_even_if_mds_config_is_missing() -> TestResult {
        let content = r##"---
        frugalos:
          segment: {}
        "##;
        let dir = track_any_err!(TempDir::new("frugalos_test"))?;
        let filepath = dir.path().join("frugalos3.yml");
        let mut file = track_any_err!(File::create(filepath.clone()))?;

        track_any_err!(file.write(content.as_bytes()))?;

        let actual = track!(FrugalosConfig::from_yaml(filepath))?;
        assert_eq!(FrugalosConfig::default(), actual);

        Ok(())
    }

    #[test]
    fn frugalos_config_value_must_not_be_unit_type() -> TestResult {
        let content = r##"---
        frugalos:
        "##;
        let dir = track_any_err!(TempDir::new("frugalos_test"))?;
        let filepath = dir.path().join("frugalos4.yml");
        let mut file = track_any_err!(File::create(filepath.clone()))?;

        track_any_err!(file.write(content.as_bytes()))?;
        assert!(FrugalosConfig::from_yaml(filepath).is_err());
        Ok(())
    }

}
