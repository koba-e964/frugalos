use fibers::Spawn;
use fibers_rpc::server::{HandleCall, Reply, ServerBuilder as RpcServerBuilder};
use libfrugalos::repair::RepairConfig;
use libfrugalos::schema::frugalos as rpc;

use ServiceHandle;

#[derive(Clone)]
pub struct RpcServer<S> {
    service_handle: ServiceHandle<S>,
}

impl<S> RpcServer<S>
where
    S: Spawn + Send + Clone + 'static,
{
    pub fn register(service_handle: ServiceHandle<S>, builder: &mut RpcServerBuilder) {
        let this = RpcServer { service_handle };
        builder.add_call_handler::<rpc::SetRepairConfigRpc, _>(this.clone());
    }
}

impl<S> HandleCall<rpc::SetRepairConfigRpc> for RpcServer<S>
where
    S: Spawn + Send + Clone + 'static,
{
    fn handle_call(&self, repair_config: RepairConfig) -> Reply<rpc::SetRepairConfigRpc> {
        self.service_handle.set_repair_config(repair_config);
        Reply::done(Ok(()))
    }
}
