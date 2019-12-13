use cannyls::device::DeviceHandle;
use fibers::Spawn;
use frugalos_mds::Event;
use frugalos_raft::NodeId;
use futures::{Async, Future, Poll, Stream};
use libfrugalos::entity::object::ObjectVersion;
use libfrugalos::repair::RepairIdleness;
use prometrics::metrics::MetricBuilder;
use slog::Logger;

use client::storage::StorageClient;
use queue_executor::general_queue_executor::GeneralQueueExecutor;
use queue_executor::repair_queue_executor::RepairQueueExecutor;
use segment_gc::{SegmentGc, SegmentGcMetrics};
use service::ServiceHandle;
use Error;

// TODO: 起動直後の確認は`device.list()`の結果を使った方が効率的
pub struct Synchronizer<S> {
    logger: Logger,
    node_id: NodeId,
    device: DeviceHandle,
    client: StorageClient<S>,
    segment_gc_metrics: SegmentGcMetrics,
    segment_gc: Option<SegmentGc>,
    segment_gc_step: u64,

    // general-purpose queue.
    general_queue: GeneralQueueExecutor,
    // repair-only queue.
    repair_queue: RepairQueueExecutor<S>,
}
impl<S> Synchronizer<S>
where
    S: Spawn + Send + Clone + 'static,
{
    pub fn new(
        logger: Logger,
        node_id: NodeId,
        device: DeviceHandle,
        service_handle: ServiceHandle<S>,
        client: StorageClient<S>,
        segment_gc_step: u64,
    ) -> Self {
        let metric_builder = MetricBuilder::new()
            .namespace("frugalos")
            .subsystem("synchronizer")
            .clone();
        // Metrics related to queue length
        let enqueued_repair = metric_builder
            .counter("enqueued_items")
            .label("type", "repair")
            .finish()
            .expect("metric should be well-formed");
        let enqueued_repair_prep = metric_builder
            .counter("enqueued_items")
            .label("type", "repair_prep")
            .finish()
            .expect("metric should be well-formed");
        let enqueued_delete = metric_builder
            .counter("enqueued_items")
            .label("type", "delete")
            .finish()
            .expect("metric should be well-formed");
        let dequeued_repair = metric_builder
            .counter("dequeued_items")
            .label("type", "repair")
            .finish()
            .expect("metric should be well-formed");
        let dequeued_repair_prep = metric_builder
            .counter("dequeued_items")
            .label("type", "repair_prep")
            .finish()
            .expect("metric should be well-formed");
        let dequeued_delete = metric_builder
            .counter("dequeued_items")
            .label("type", "delete")
            .finish()
            .expect("metric should be well-formed");

        let general_queue = GeneralQueueExecutor::new(
            &logger,
            node_id,
            &device,
            &enqueued_repair_prep,
            &enqueued_delete,
            &dequeued_repair_prep,
            &dequeued_delete,
        );
        let repair_queue = RepairQueueExecutor::new(
            &logger,
            node_id,
            &device,
            &client,
            &service_handle,
            &metric_builder,
            &enqueued_repair,
            &dequeued_repair,
        );
        Synchronizer {
            logger,
            node_id,
            device,
            client,
            segment_gc_metrics: SegmentGcMetrics::new(&metric_builder),
            segment_gc: None,
            segment_gc_step,

            general_queue,
            repair_queue,
        }
    }
    pub fn handle_event(&mut self, event: &Event) {
        debug!(
            self.logger,
            "New event: {:?} (metadata={})",
            event,
            self.client.is_metadata(),
        );
        if !self.client.is_metadata() {
            match *event {
                Event::Putted { .. } => {
                    self.general_queue.push(event);
                }
                Event::Deleted { .. } => {
                    self.general_queue.push(event);
                }
                // Because pushing FullSync into the task queue causes difficulty in implementation,
                // we decided not to push this task to the task priority queue and handle it manually.
                Event::FullSync {
                    ref machine,
                    next_commit,
                } => {
                    // If FullSync is not being processed now, this event lets the synchronizer to handle one.
                    if self.segment_gc.is_none() {
                        self.segment_gc = Some(SegmentGc::new(
                            &self.logger,
                            self.node_id,
                            &self.device,
                            machine.clone(),
                            ObjectVersion(next_commit.as_u64()),
                            self.segment_gc_metrics.clone(),
                            self.segment_gc_step,
                        ));
                    }
                }
            }
        }
    }
    pub(crate) fn set_repair_idleness_threshold(
        &mut self,
        repair_idleness_threshold: RepairIdleness,
    ) {
        self.repair_queue
            .set_repair_idleness_threshold(repair_idleness_threshold);
    }
}
impl<S> Future for Synchronizer<S>
where
    S: Spawn + Send + Clone + 'static,
{
    type Item = ();
    type Error = Error;
    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        while let Async::Ready(Some(())) = self.segment_gc.poll().unwrap_or_else(|e| {
            warn!(self.logger, "Task failure: {}", e);
            Async::Ready(Some(()))
        }) {
            // Full sync is done. Clearing the segment_gc field.
            self.segment_gc = None;
            self.segment_gc_metrics.reset();
        }

        if let Async::Ready(Some(versions)) = self.general_queue.poll().unwrap_or_else(|e| {
            warn!(self.logger, "Task failure in general_queue: {}", e);
            Async::Ready(None)
        }) {
            for version in versions {
                self.repair_queue.push(version);
            }
        }

        // Never stops, never fails.
        self.repair_queue.poll().unwrap_or_else(Into::into);
        Ok(Async::NotReady)
    }
}
