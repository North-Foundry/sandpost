use super::*;

/// Convert literal arguments into the owned strings the process supplies.
fn arguments(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

/// Every supported spelling selects its command, and no arguments select the runtime.
#[test]
fn supported_commands_parse() {
    for (values, expected) in [
        (&[][..], None),
        (
            &["search", "status"][..],
            Some(MaintenanceCommand::SearchStatus),
        ),
        (
            &["search", "rebuild"][..],
            Some(MaintenanceCommand::SearchRebuild),
        ),
        (
            &["migrate"][..],
            Some(MaintenanceCommand::Migrate(MigrationOperation::Up)),
        ),
        (
            &["migrate", "up"][..],
            Some(MaintenanceCommand::Migrate(MigrationOperation::Up)),
        ),
        (
            &["migrate", "status"][..],
            Some(MaintenanceCommand::Migrate(MigrationOperation::Status)),
        ),
        (
            &["migrate", "rollback"][..],
            Some(MaintenanceCommand::Migrate(MigrationOperation::Rollback)),
        ),
    ] {
        assert_eq!(
            MaintenanceCommand::parse(&arguments(values)).unwrap(),
            expected,
            "{values:?}"
        );
    }
}

/// Unknown commands report the complete usage; unknown migration operations report theirs.
#[test]
fn unsupported_commands_report_usage() {
    for (values, usage) in [
        (&["search"][..], USAGE),
        (&["search", "status", "extra"][..], USAGE),
        (&["serve"][..], USAGE),
        (&["migrate", "down"][..], MIGRATION_USAGE),
        (&["migrate", "up", "extra"][..], MIGRATION_USAGE),
    ] {
        let error = MaintenanceCommand::parse(&arguments(values)).unwrap_err();
        assert_eq!(error.to_string(), usage, "{values:?}");
    }
}
