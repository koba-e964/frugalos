#![allow(clippy::ptr_arg)]
use fibers::Spawn;
use fibers_rpc::client::ClientServiceHandle as RpcServiceHandle;
use frugalos_segment::config::ClusterMember;
use frugalos_segment::Client as Segment;
use frugalos_segment::{self, ErasureCoder, FrugalosSegmentConfig};
use libfrugalos::entity::bucket::Bucket as BucketConfig;
use libfrugalos::entity::object::ObjectId;
use siphasher;
use slog::Logger;
use std::iter;

use Result;

#[derive(Clone)]
pub struct Bucket<S> {
    logger: Logger,
    rpc_service: RpcServiceHandle,
    ec: Option<ErasureCoder>,
    storage_config: frugalos_segment::config::Storage,
    segment_config: FrugalosSegmentConfig,
    segments: Vec<Segment<S>>,
}
impl<S> Bucket<S>
where
    S: Spawn + Send + 'static,
{
    pub fn new(
        logger: Logger,
        rpc_service: RpcServiceHandle,
        config: &BucketConfig,
        segment_config: FrugalosSegmentConfig,
        spawner: S,
    ) -> Result<Self> {
        let ec = match config {
            BucketConfig::Metadata(_) => None,
            BucketConfig::Replicated(_) => None,
            BucketConfig::Dispersed(ref c) => Some(frugalos_segment::build_ec(
                c.data_fragment_count as usize,
                c.tolerable_faults as usize,
            )),
        };

        let storage_config = match config {
            BucketConfig::Metadata(_) => frugalos_segment::config::Storage::Metadata,
            BucketConfig::Replicated(ref b) => {
                let c = frugalos_segment::config::ReplicatedConfig {
                    tolerable_faults: b.tolerable_faults as u8,
                };
                frugalos_segment::config::Storage::Replicated(c)
            }
            BucketConfig::Dispersed(ref b) => {
                let c = frugalos_segment::config::DispersedConfig {
                    tolerable_faults: b.tolerable_faults as u8,
                    fragments: (b.tolerable_faults + b.data_fragment_count) as u8,
                };
                frugalos_segment::config::Storage::Dispersed(c)
            }
        };

        let client_config = frugalos_segment::config::ClientConfig {
            cluster: frugalos_segment::config::ClusterConfig {
                members: Vec::new(),
            },
            dispersed_client: segment_config.dispersed_client.clone(),
            replicated_client: segment_config.replicated_client.clone(),
            storage: storage_config.clone(),
            mds: segment_config.mds_client.clone(),
        };
        let segment = track!(Segment::new(
            logger.clone(),
            rpc_service.clone(),
            client_config,
            ec.clone(),
            spawner,
        ))?;
        let segments = iter::repeat(segment)
            .take(config.segment_count() as usize)
            .collect();
        Ok(Bucket {
            logger,
            rpc_service,
            ec,
            storage_config,
            segments,
            segment_config,
        })
    }
    pub fn update_segment(
        &mut self,
        segment_no: u16,
        members: Vec<ClusterMember>,
        spawner: S,
    ) -> Result<()> {
        let segment_config = frugalos_segment::config::ClientConfig {
            cluster: frugalos_segment::config::ClusterConfig { members },
            dispersed_client: self.segment_config.dispersed_client.clone(),
            replicated_client: self.segment_config.replicated_client.clone(),
            storage: self.storage_config.clone(),
            mds: self.segment_config.mds_client.clone(),
        };
        let segment = track!(Segment::new(
            self.logger.clone(),
            self.rpc_service.clone(),
            segment_config,
            self.ec.clone(),
            spawner,
        ))?;
        self.segments[segment_no as usize] = segment;
        Ok(())
    }
    pub fn get_segment(&self, id: &ObjectId) -> &Segment<S> {
        use std::hash::{Hash, Hasher};
        let mut hasher = siphasher::sip::SipHasher13::new();
        id.hash(&mut hasher);
        let i = hasher.finish() as usize % self.segments.len();
        &self.segments[i]
    }
    pub fn segments(&self) -> &[Segment<S>] {
        &self.segments
    }
}
