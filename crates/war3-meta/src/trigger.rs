//! Trigger definitions, loaded from `UI\TriggerData.txt` and
//! `UI\TriggerStrings.txt`.
//!
//! # Why these files are mandatory
//!
//! The parameter count of a trigger call is **not recorded in the `.wtg` file**.
//! It has to be recomputed from the GUI definitions, and parameters of type
//! `nothing` are skipped entirely when reading and when writing. Without the
//! trigger definitions, a `.wtg` can be neither read nor written.
//!
//! Those files exist only in a game installation, so loading goes through an
//! [`war3_core::AssetSource`] rather than shipping a copy.
//!
//! # Field layout
//!
//! ```text
//! [TriggerTypes]   typename = field1, field2, field3[, field4...]
//! [TriggerParams]  key      = field1, field2 (type name), field3 (value), field4 (display)
//! ```
//!
//! Both paths matter. A **preset** parameter stores a name that is looked up in
//! `TriggerParams` to find its type, while a **constant** parameter stores a
//! literal whose type comes from the action definition's parameter slot. Handling
//! only one of the two is wrong.

use std::collections::BTreeMap;
use std::fmt;

use war3_core::diag::{Diagnostic, DiagnosticCode, Diagnostics};

/// One `[TriggerTypes]` entry.
///
/// The format is `typename = field1, field2, field3[, field4...]`, with no
/// dedicated display-name field.
///
/// Fields from the fourth onwards are ambiguous. In practice the fourth is a
/// display name — either a `WESTRING_*` key or literal text — and a fifth, when
/// present, is a type name that sometimes repeats the entry's own name. Treating
/// the fourth field as a type is therefore wrong; it is kept verbatim here and
/// [`TriggerType::candidate_base_type`] offers a heuristic for callers that want
/// the related type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerType {
    /// The key, which is a JASS type name such as `unit` or `abilcode`.
    pub name: String,
    /// Field 1, a 0 or 1 flag of undetermined meaning.
    pub field1: i32,
    /// Field 2, which is 1 throughout the files seen.
    pub field2: i32,
    /// Field 3, a 0 or 1 flag of undetermined meaning.
    pub field3: i32,
    /// Field 4 and beyond, kept verbatim. May contain display text.
    pub extra: Vec<String>,
}

impl TriggerType {
    /// Whether no related type could be identified.
    #[must_use]
    pub fn is_self_contained(&self) -> bool {
        self.candidate_base_type().is_none()
    }

    /// A heuristic guess at the related type.
    ///
    /// The last extra field is taken as the related type unless it is a
    /// `WESTRING_*` key or the entry's own name. This is a heuristic, not a
    /// guarantee; use [`TriggerType::extra`] directly when certainty is needed.
    #[must_use]
    pub fn candidate_base_type(&self) -> Option<&str> {
        let last = self.extra.last()?;
        if last.starts_with("WESTRING_") || last == &self.name {
            return None;
        }
        Some(last.as_str())
    }

    /// Whether the candidate related type is itself a defined type name.
    ///
    /// Without this check, literal display text such as a Chinese type label
    /// sitting in the last field would be mistaken for a type.
    #[must_use]
    pub fn candidate_is_known_type(&self, known: &BTreeMap<String, TriggerType>) -> bool {
        self.candidate_base_type().is_some_and(|t| known.contains_key(t))
    }
}

/// One `[TriggerParams]` entry, which is a **preset value**.
///
/// A `.wtg` file stores the *key* of this entry, not its value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerParam {
    /// The preset name, as stored in the `.wtg`.
    pub name: String,
    /// Field 1, a flag of undetermined meaning.
    pub field1: i32,
    /// Field 2: the type name, matching a [`TriggerType::name`].
    ///
    /// This column is the only thing needed to resolve a preset's type.
    pub type_name: String,
    /// Field 3: the actual value. May be a number, a quoted string, a JASS
    /// expression, a constant name, or `true`/`false`.
    pub value: String,
    /// Field 4: display text, which may be a `WESTRING_*` key or literal text.
    ///
    /// This column can carry display text, so it must not be distributed with
    /// the repository.
    pub display: Option<String>,
}

/// A GUI event, condition, action or call definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerFunction {
    /// The key, which is the JASS function name.
    pub name: String,
    /// Parameter slots, which determine how many records a `.wtg` holds.
    pub args: Vec<TriggerArg>,
    /// Return type; only call entries have one.
    pub returns: Option<String>,
}

impl TriggerFunction {
    /// How many parameter records a `.wtg` writes for this function.
    ///
    /// Parameters of type `nothing` are skipped entirely, by both the reader and
    /// the writer.
    #[must_use]
    pub fn serialized_arg_count(&self) -> usize {
        self.args.iter().filter(|a| a.type_name != "nothing").count()
    }

    /// The parameter at a given position among those that are serialised.
    #[must_use]
    pub fn serialized_arg(&self, index: usize) -> Option<&TriggerArg> {
        self.args
            .iter()
            .filter(|a| a.type_name != "nothing")
            .nth(index)
    }
}

/// One parameter slot of an event or action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerArg {
    /// The type name, matching a [`TriggerType::name`].
    pub type_name: String,
    /// Default value, as a JASS expression.
    pub default: Option<String>,
    /// Lower bound, present only in the LNI-style definitions.
    pub min: Option<i64>,
    /// Upper bound, present only in the LNI-style definitions.
    pub max: Option<i64>,
}

/// The full set of GUI trigger definitions.
#[derive(Debug, Clone, Default)]
pub struct TriggerData {
    /// `[TriggerTypes]`.
    pub types: BTreeMap<String, TriggerType>,
    /// `[TriggerParams]`, keyed by preset name.
    pub params: BTreeMap<String, TriggerParam>,
    /// `[TriggerEvents]`.
    pub events: BTreeMap<String, TriggerFunction>,
    /// `[TriggerConditions]`.
    pub conditions: BTreeMap<String, TriggerFunction>,
    /// `[TriggerActions]`.
    pub actions: BTreeMap<String, TriggerFunction>,
    /// `[TriggerCalls]`.
    pub calls: BTreeMap<String, TriggerFunction>,
    /// Diagnostics from loading.
    pub diagnostics: Diagnostics,
    /// Whether nothing was loaded.
    pub empty: bool,
}

impl TriggerData {
    /// An empty set, which is what a machine without the game yields.
    ///
    /// This is a legitimate state, not an error.
    #[must_use]
    pub fn empty() -> Self {
        Self { empty: true, ..Default::default() }
    }

    /// Whether the definitions are usable.
    #[must_use]
    pub fn is_usable(&self) -> bool {
        !self.empty
            && !(self.actions.is_empty() && self.events.is_empty() && self.conditions.is_empty())
    }

    /// Looks up an action, event, condition or call by name.
    #[must_use]
    pub fn function(&self, name: &str) -> Option<&TriggerFunction> {
        self.actions
            .get(name)
            .or_else(|| self.events.get(name))
            .or_else(|| self.conditions.get(name))
            .or_else(|| self.calls.get(name))
    }

    /// Resolves the type of a `preset` parameter from its stored name.
    ///
    /// This is the one step a `.wtg` parser cannot do without: preset parameters
    /// store a name, and the type comes from field 2 of `TriggerParams`.
    ///
    /// An unknown preset name is reported rather than passed over, because a
    /// silent miss would produce a wrongly typed parameter.
    pub fn preset_type(&self, preset_name: &str, diagnostics: &mut Diagnostics) -> Option<String> {
        match self.params.get(preset_name) {
            Some(param) => Some(param.type_name.clone()),
            None => {
                diagnostics.push(Diagnostic::warn(
                    DiagnosticCode::AssetFallbackUsed,
                    format!(
                        "preset parameter {preset_name:?} is absent from TriggerParams, so its type \
                         cannot be determined"
                    ),
                ));
                None
            }
        }
    }

    /// Loads the definitions from an asset source.
    ///
    /// Returns an empty set with a diagnostic when the files are missing, rather
    /// than failing: the rest of a map can still be parsed without them.
    #[must_use]
    pub fn load_from_assets(source: &dyn war3_core::AssetSource) -> Self {
        let mut diagnostics = Diagnostics::new();

        let Some(text) = source.get_text("UI\\TriggerData.txt") else {
            diagnostics.push(Diagnostic::info(
                DiagnosticCode::AssetFallbackUsed,
                "UI\\TriggerData.txt is unavailable, so trigger definitions cannot be read; \
                 .wtg and .wct cannot be parsed without them",
            ));
            return Self { diagnostics, empty: true, ..Default::default() };
        };

        let mut data = Self::parse_ini(&text);
        data.diagnostics.merge(&diagnostics);
        data.empty = false;

        match source.get_text("UI\\TriggerStrings.txt") {
            Some(_) => {
                // Existence is all that matters here; the text itself belongs to
                // the expressive layer and is not stored.
                data.diagnostics.push(Diagnostic::info(
                    DiagnosticCode::AssetFallbackUsed,
                    "TriggerStrings.txt is present; its text is display-layer data and is not stored",
                ));
            }
            None => {
                data.diagnostics.push(Diagnostic::info(
                    DiagnosticCode::AssetFallbackUsed,
                    "UI\\TriggerStrings.txt is unavailable, so trigger display text degrades to \
                     WESTRING_* keys",
                ));
            }
        }
        data
    }

    /// Parses the sections of `TriggerData.txt` that matter.
    ///
    /// The `*Strings`, `*Defaults`, `*Limits`, `*Category`, `*ScriptName`,
    /// `*UseWithAI` and `*AIDefaults` sections hold attributes of the entries
    /// rather than parsing data, and are skipped.
    #[must_use]
    pub fn parse_ini(text: &str) -> Self {
        let mut data = Self::default();
        let mut section = String::new();

        for raw_line in text.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with("//") {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                section = name.trim().to_string();
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim();

            match section.as_str() {
                "TriggerTypes" => {
                    let fields: Vec<&str> = value.split(',').map(str::trim).collect();
                    let type_def = TriggerType {
                        name: key.to_string(),
                        field1: parse_i32(fields.first().copied()),
                        field2: parse_i32(fields.get(1).copied()),
                        field3: parse_i32(fields.get(2).copied()),
                        // Field 4 onwards is kept verbatim; it may be a display
                        // name rather than a type.
                        extra: fields.iter().skip(3).map(|s| (*s).to_string()).collect(),
                    };
                    data.types.insert(key.to_string(), type_def);
                }
                "TriggerParams" => {
                    let fields: Vec<&str> = value.split(',').map(str::trim).collect();
                    let param = TriggerParam {
                        name: key.to_string(),
                        field1: parse_i32(fields.first().copied()),
                        type_name: fields.get(1).map_or(String::new(), |s| (*s).to_string()),
                        value: fields.get(2).map_or(String::new(), |s| (*s).to_string()),
                        display: fields.get(3).map(|s| (*s).to_string()),
                    };
                    data.params.insert(key.to_string(), param);
                }
                "TriggerEvents" | "TriggerConditions" | "TriggerActions" | "TriggerCalls" => {
                    // Format: `Name=argtype1,argtype2,...`. Call entries carry two
                    // extra leading fields, `use_in_event` and the return type.
                    let fields: Vec<&str> = value.split(',').map(str::trim).collect();
                    let (returns, arg_start) = if section == "TriggerCalls" {
                        (fields.get(1).map(|s| (*s).to_string()), 2usize)
                    } else {
                        (None, 0usize)
                    };
                    let args = fields
                        .iter()
                        .skip(arg_start)
                        .filter(|s| !s.is_empty())
                        .map(|t| TriggerArg {
                            type_name: (*t).to_string(),
                            default: None,
                            min: None,
                            max: None,
                        })
                        .collect();
                    let function = TriggerFunction { name: key.to_string(), args, returns };
                    match section.as_str() {
                        "TriggerEvents" => {
                            data.events.insert(key.to_string(), function);
                        }
                        "TriggerConditions" => {
                            data.conditions.insert(key.to_string(), function);
                        }
                        "TriggerActions" => {
                            data.actions.insert(key.to_string(), function);
                        }
                        _ => {
                            data.calls.insert(key.to_string(), function);
                        }
                    }
                }
                _ => {}
            }
        }

        if data.types.is_empty() && data.params.is_empty() {
            data.diagnostics.push(Diagnostic::warn(
                DiagnosticCode::AssetFallbackUsed,
                "TriggerData.txt yielded no TriggerTypes or TriggerParams sections",
            ));
        }
        data
    }
}

impl fmt::Display for TriggerData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.empty {
            return f.write_str("TriggerData(unavailable)");
        }
        write!(
            f,
            "TriggerData({} types, {} presets, {} events, {} conditions, {} actions, {} calls)",
            self.types.len(),
            self.params.len(),
            self.events.len(),
            self.conditions.len(),
            self.actions.len(),
            self.calls.len()
        )
    }
}

fn parse_i32(s: Option<&str>) -> i32 {
    s.and_then(|v| v.trim().parse::<i32>().ok()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
// trigger definitions
[TriggerTypes]
boolean=0,1,1,WESTRING_TRIGTYPE_boolean
integer=0,1,1,WESTRING_TRIGTYPE_integer
real=0,1,1,WESTRING_TRIGTYPE_real
unitcode=0,1,1,WESTRING_TRIGTYPE_unitcode,integer
radian=0,1,1,degrees,real
handle=0,0,0,WESTRING_TRIGTYPE_handle

[TriggerParams]
Player00=0,player,Player(0),WESTRING_PLAYER_00
OperatorAdd=0,ArithmeticOperator,\"+\",WESTRING_ARITHMETICOPERATOR_ADD

[TriggerActions]
SetUnitPosition=unit,real,real
DoNothing=
SomeAction=nothing,unit

[TriggerEvents]
MapInitializationEvent=
TimeOfDayEvent=real

[TriggerCalls]
CreateUnit=,unit,player,integer,real,real,real
";

    #[test]
    fn parses_trigger_types() {
        let data = TriggerData::parse_ini(SAMPLE);
        assert_eq!(data.types.len(), 6);
        let unitcode = data.types.get("unitcode").unwrap();
        assert_eq!(unitcode.field1, 0);
        assert_eq!(unitcode.field2, 1);
        assert_eq!(unitcode.field3, 1);
        // Field 4 is a display name and field 5 is the type; treating the fourth
        // as the type is wrong.
        assert_eq!(
            unitcode.extra,
            vec!["WESTRING_TRIGTYPE_unitcode".to_string(), "integer".to_string()]
        );
        assert_eq!(unitcode.candidate_base_type(), Some("integer"));
        assert!(unitcode.candidate_is_known_type(&data.types));
    }

    #[test]
    fn a_literal_display_name_is_not_mistaken_for_a_type() {
        let data = TriggerData::parse_ini(SAMPLE);
        let radian = data.types.get("radian").unwrap();
        assert_eq!(radian.extra, vec!["degrees".to_string(), "real".to_string()]);
        assert_eq!(radian.candidate_base_type(), Some("real"));
        assert!(radian.candidate_is_known_type(&data.types));
        assert!(!radian.is_self_contained());
    }

    #[test]
    fn a_westring_only_entry_has_no_candidate_base_type() {
        let data = TriggerData::parse_ini(SAMPLE);
        let boolean = data.types.get("boolean").unwrap();
        assert_eq!(boolean.extra, vec!["WESTRING_TRIGTYPE_boolean".to_string()]);
        assert!(boolean.is_self_contained());
        assert_eq!(boolean.candidate_base_type(), None);
    }

    #[test]
    fn a_type_with_no_extra_fields_is_self_contained() {
        let data = TriggerData::parse_ini("[TriggerTypes]\nagent=0,0,0\n");
        let agent = data.types.get("agent").unwrap();
        assert!(agent.extra.is_empty());
        assert!(agent.is_self_contained());
    }

    #[test]
    fn a_candidate_that_is_not_a_known_type_is_rejected() {
        let data = TriggerData::parse_ini("[TriggerTypes]\nfoo=0,1,1,some display text\n");
        let foo = data.types.get("foo").unwrap();
        assert_eq!(foo.candidate_base_type(), Some("some display text"));
        assert!(!foo.candidate_is_known_type(&data.types));
    }

    #[test]
    fn parses_trigger_params_and_keeps_field_two_as_the_type() {
        let data = TriggerData::parse_ini(SAMPLE);
        assert_eq!(data.params.len(), 2);
        let p = data.params.get("Player00").unwrap();
        assert_eq!(p.type_name, "player");
        assert_eq!(p.value, "Player(0)");
        assert_eq!(p.display.as_deref(), Some("WESTRING_PLAYER_00"));
    }

    #[test]
    fn quoted_param_values_are_preserved_verbatim() {
        let data = TriggerData::parse_ini(SAMPLE);
        let op = data.params.get("OperatorAdd").unwrap();
        // The quotes are part of a JASS string literal and must not be stripped.
        assert_eq!(op.value, "\"+\"");
    }

    #[test]
    fn nothing_args_are_excluded_from_the_serialized_count() {
        // Which is exactly why the parameter count is not in the .wtg file.
        let data = TriggerData::parse_ini(SAMPLE);
        let some = data.function("SomeAction").unwrap();
        assert_eq!(some.args.len(), 2);
        assert_eq!(some.serialized_arg_count(), 1, "nothing args must be skipped");
        assert_eq!(some.serialized_arg(0).unwrap().type_name, "unit");
        assert!(some.serialized_arg(1).is_none());
    }

    #[test]
    fn zero_arg_functions_serialize_no_args() {
        let data = TriggerData::parse_ini(SAMPLE);
        assert_eq!(data.function("DoNothing").unwrap().serialized_arg_count(), 0);
        assert_eq!(
            data.function("MapInitializationEvent").unwrap().serialized_arg_count(),
            0
        );
    }

    #[test]
    fn call_entries_drop_the_first_two_comma_fields() {
        let data = TriggerData::parse_ini(SAMPLE);
        let call = data.calls.get("CreateUnit").unwrap();
        assert_eq!(call.returns.as_deref(), Some("unit"));
        assert_eq!(call.args.len(), 5);
        assert_eq!(call.args[0].type_name, "player");
    }

    #[test]
    fn preset_type_lookup_uses_field_two() {
        let data = TriggerData::parse_ini(SAMPLE);
        let mut diags = Diagnostics::new();
        assert_eq!(data.preset_type("Player00", &mut diags).as_deref(), Some("player"));
        assert!(diags.is_empty());
    }

    #[test]
    fn unknown_preset_is_diagnosed_not_silently_defaulted() {
        let data = TriggerData::parse_ini(SAMPLE);
        let mut diags = Diagnostics::new();
        assert_eq!(data.preset_type("NoSuchPreset", &mut diags), None);
        assert_eq!(diags.len(), 1);
        assert!(diags.items()[0].message.contains("NoSuchPreset"));
    }

    #[test]
    fn function_lookup_searches_all_four_sections() {
        let data = TriggerData::parse_ini(SAMPLE);
        assert!(data.function("SetUnitPosition").is_some());
        assert!(data.function("MapInitializationEvent").is_some());
        assert!(data.function("CreateUnit").is_some());
        assert!(data.function("Nope").is_none());
    }

    #[test]
    fn missing_game_data_yields_an_empty_but_reported_state() {
        let data = TriggerData::load_from_assets(&war3_core::EmptyAssetSource);
        assert!(data.empty);
        assert!(!data.is_usable());
        assert!(data
            .diagnostics
            .items()
            .iter()
            .any(|d| d.message.contains("TriggerData.txt")));
        // A missing installation is informational, not an error.
        assert!(!data.diagnostics.has_problems());
    }

    #[test]
    fn loads_from_a_memory_asset_source() {
        let source = war3_core::MemoryAssetSource::new()
            .with("UI\\TriggerData.txt", SAMPLE.as_bytes().to_vec());
        let data = TriggerData::load_from_assets(&source);
        assert!(data.is_usable());
        assert_eq!(data.actions.len(), 3);
        assert_eq!(data.events.len(), 2);
    }

    #[test]
    fn comments_and_blank_lines_are_skipped() {
        let data = TriggerData::parse_ini("\n// comment\n[TriggerTypes]\n\ninteger=0,1,1\n");
        assert_eq!(data.types.len(), 1);
    }

    #[test]
    fn malformed_numeric_fields_degrade_to_zero() {
        let data = TriggerData::parse_ini("[TriggerTypes]\nfoo=abc,def\n");
        let t = data.types.get("foo").unwrap();
        assert_eq!(t.field1, 0);
        assert_eq!(t.field2, 0);
    }

    #[test]
    fn empty_input_is_diagnosed() {
        let data = TriggerData::parse_ini("");
        assert!(data.diagnostics.has_problems());
    }
}
