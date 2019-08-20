//! Definitions for frugalos set-repair-config
use clap::{App, Arg, ArgMatches, SubCommand};
use libfrugalos::repair::{RepairConfig, RepairIdleness};
use sloggers::Build;
use sloggers::LoggerBuilder;
use std::net::ToSocketAddrs;
use std::time::Duration;
use trackable::error::ErrorKindExt;

use command::{default_rpc_server_bind_addr, warn_if_there_are_unknown_fields, FrugalosSubcommand};
use {Error, ErrorKind};

/// frugalos set-repair-config
pub struct SetRepairConfigCommand;

static REPAIR_IDLENESS_THRESHOLD: &str = "REPAIR_IDLENESS_THRESHOLD";
static REPAIR_IDLENESS_THRESHOLD_LONG_ARG: &str = "repair-idleness-threshold";
static DISABLE_REPAIR_IDLENESS: &str = "DISABLE_REPAIR_IDLENESS";
static DISABLE_REPAIR_IDLENESS_LONG_ARG: &str = "disable-repair-idleness";

impl FrugalosSubcommand for SetRepairConfigCommand {
    fn get_subcommand<'a, 'b: 'a>(&self) -> App<'a, 'b> {
        SubCommand::with_name("set-repair-config")
            .arg(
                Arg::with_name("RPC_ADDR")
                    .long("rpc-addr")
                    .takes_value(true)
                    .default_value(default_rpc_server_bind_addr()),
            )
            .arg(
                Arg::with_name(REPAIR_IDLENESS_THRESHOLD)
                    .long(REPAIR_IDLENESS_THRESHOLD_LONG_ARG)
                    .takes_value(true),
            )
            .arg(
                Arg::with_name(DISABLE_REPAIR_IDLENESS)
                    .long(DISABLE_REPAIR_IDLENESS_LONG_ARG)
                    .takes_value(false),
            )
    }

    fn check_matches<'a>(&self, matches: &'a ArgMatches<'a>) -> Option<&'a ArgMatches<'a>> {
        matches.subcommand_matches("set-repair-config")
    }

    fn handle_matches(
        &self,
        logger_builder: LoggerBuilder,
        matches: &ArgMatches,
        unknown_fields: &[String],
    ) {
        // Set repair_idleness_threshold
        let mut logger = track_try_unwrap!(logger_builder.build());
        warn_if_there_are_unknown_fields(&mut logger, &unknown_fields);
        let mut rpc_addrs = track_try_unwrap!(track_any_err!(matches
            .value_of("RPC_ADDR")
            .unwrap()
            .to_socket_addrs()));
        let repair_config = Self::get_repair_config_from_matches(matches);
        let rpc_addr = rpc_addrs.nth(0).expect("No available TCP address");
        let logger = logger.new(o!("rpc_addr" => rpc_addr.to_string(),
            "repair_config" => format!("{:?}", repair_config)));
        track_try_unwrap!(crate::daemon::set_repair_config(
            &logger,
            rpc_addr,
            repair_config,
        ));
    }
}

impl SetRepairConfigCommand {
    fn get_repair_config_from_matches(matches: &ArgMatches) -> RepairConfig {
        let repair_idleness_threshold: Option<RepairIdleness>;
        // if DISABLE_REPAIR_IDLENESS is present, REPAIR_IDLENESS_THRESHOLD is ignored.
        if matches.is_present(DISABLE_REPAIR_IDLENESS) {
            repair_idleness_threshold = Some(RepairIdleness::Disabled);
        } else {
            repair_idleness_threshold = matches.value_of(REPAIR_IDLENESS_THRESHOLD).map(|str| {
                let duration_secs: f64 = track_try_unwrap!(str.parse().map_err(|_| Error::from(
                    ErrorKind::InvalidInput.cause("repair-idleness-threshold must be a float")
                )));
                // TODO check if duration_secs is non-negative
                RepairIdleness::Threshold(Duration::from_millis((duration_secs * 1000.0) as u64))
            })
        }
        // TODO: accept repair_concurrency_limit and segment_gc_concurrency_limit
        RepairConfig {
            repair_concurrency_limit: None,
            repair_idleness_threshold,
            segment_gc_concurrency_limit: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::App;
    use libfrugalos::repair::{RepairConfig, RepairIdleness};
    use std::time::Duration;

    use super::SetRepairConfigCommand;
    use command::FrugalosSubcommand;

    #[test]
    fn get_repair_config_from_matches_work_correctly() {
        let set_repair_config_command = SetRepairConfigCommand;
        let matches = App::new("frugalos-test")
            .subcommand(set_repair_config_command.get_subcommand())
            .get_matches_from(vec![
                "frugalos-test",
                "set-repair-config",
                "--repair-idleness-threshold",
                "4.0",
            ]);
        if let Some(matches) = set_repair_config_command.check_matches(&matches) {
            let repair_config = SetRepairConfigCommand::get_repair_config_from_matches(&matches);
            // TODO: we want to check repair_config directly, but it's not possible because RepairConfig doesn't implement Eq.
            // To circumvent this, we perform pattern matching.
            eprintln!("{:?}", repair_config);
            if let RepairConfig {
                repair_concurrency_limit: None,
                repair_idleness_threshold: Some(RepairIdleness::Threshold(duration)),
                segment_gc_concurrency_limit: None,
            } = repair_config
            {
                assert_eq!(duration, Duration::from_secs(4))
            } else {
                panic!();
            }
        }
    }
    #[test]
    fn get_repair_config_from_matches_work_correctly_disable() {
        let set_repair_config_command = SetRepairConfigCommand;
        let matches = App::new("frugalos-test")
            .subcommand(set_repair_config_command.get_subcommand())
            .get_matches_from(vec![
                "frugalos-test",
                "set-repair-config",
                "--disable-repair-idleness", // This option neutralizes --repair-idleness-threshold that appears later
                "--repair-idleness-threshold",
                "10.0",
            ]);
        if let Some(matches) = set_repair_config_command.check_matches(&matches) {
            let repair_config = SetRepairConfigCommand::get_repair_config_from_matches(&matches);
            // TODO: we want to check repair_config directly, but it's not possible because RepairConfig doesn't implement Eq.
            // To circumvent this, we perform pattern matching.
            if let RepairConfig {
                repair_concurrency_limit: None,
                repair_idleness_threshold: Some(RepairIdleness::Disabled),
                segment_gc_concurrency_limit: None,
            } = repair_config
            {
            } else {
                panic!();
            }
        }
    }
}
