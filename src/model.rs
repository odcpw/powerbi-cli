use crate::calculated_columns::calculated_columns_command;
use crate::measures::measures_command;
use crate::model_advanced::advanced_model_command;
use crate::model_dax::dax_command;
use crate::model_live::live_model_command;
use crate::partitions::partitions_command;
use crate::relationships::relationships_command;
use crate::static_tables::static_tables_command;
use crate::{CliError, CliResult};
use serde_json::Value;

pub(crate) fn model_command(args: &[String]) -> CliResult<Value> {
    let Some((family, rest)) = args.split_first() else {
        return Err(CliError::invalid_args(
            "model requires a subcommand: measures, calculated-columns, relationships, partitions, tables, dax, live",
        )
        .with_hint("Run `powerbi-cli model measures list --project <project-dir-or.pbip> --json`.")
        .with_suggested_command(
            "powerbi-cli model measures list --project <project-dir-or.pbip> --json",
        ));
    };

    match family.as_str() {
        "calculated-column" | "calculated-columns" | "calculatedColumn" | "calculatedColumns" => {
            calculated_columns_command(rest)
        }
        "dax" => dax_command(rest),
        "live" => live_model_command(rest),
        "role" | "roles" | "rls" => advanced_model_command("roles", rest),
        "perspective" | "perspectives" => advanced_model_command("perspectives", rest),
        "culture" | "cultures" | "translation" | "translations" => {
            advanced_model_command("cultures", rest)
        }
        "expression" | "expressions" | "named-expression" | "named-expressions" => {
            advanced_model_command("expressions", rest)
        }
        "advanced" => match rest.split_first() {
            Some((action, tail)) if action == "inventory" => {
                advanced_model_command("inventory", tail)
            }
            _ => advanced_model_command("inventory", rest),
        },
        "inventory" => advanced_model_command("inventory", rest),
        "measure" | "measures" => measures_command(rest),
        "partition" | "partitions" => partitions_command(rest),
        "relationship" | "relationships" => relationships_command(rest),
        "table" | "tables" => static_tables_command(rest),
        _ => Err(CliError::invalid_args(format!("unknown model command family: {family}"))
            .with_hint("Run `powerbi-cli --json capabilities --for model` for supported model commands.")
            .with_suggested_command("powerbi-cli --json capabilities --for model")),
    }
}
