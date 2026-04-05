#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolClass {
    Identity,
    FollowUp,
    SearchStack,
    ExaSearch,
    LightCompute,
    ManageApp,
    InspectApp,
    ScratchThought,
    ScratchpadSearch,
    ScratchpadEdit,
    StackProposal,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepOutcome {
    Terminal,
    NonTerminal,
    StructuralAnomaly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverOutcome {
    Answered,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeledTurn {
    pub tool: ToolClass,
    pub has_tool_call: bool,
    pub multiple_tool_calls: bool,
    pub forced_terminal: bool,
    pub assistant_content_present: bool,
    pub arguments_valid: bool,
    pub execution_successful: bool,
}

pub const fn modeled_outcome(tool: ToolClass, forced_terminal: bool) -> StepOutcome {
    if forced_terminal {
        return match tool {
            ToolClass::Identity => StepOutcome::Terminal,
            _ => StepOutcome::StructuralAnomaly,
        };
    }

    match tool {
        ToolClass::Identity => StepOutcome::Terminal,
        ToolClass::FollowUp
        | ToolClass::SearchStack
        | ToolClass::ExaSearch
        | ToolClass::LightCompute
        | ToolClass::ManageApp
        | ToolClass::InspectApp
        | ToolClass::ScratchThought
        | ToolClass::ScratchpadSearch
        | ToolClass::ScratchpadEdit
        | ToolClass::StackProposal => StepOutcome::NonTerminal,
        ToolClass::Unknown => StepOutcome::StructuralAnomaly,
    }
}

pub const fn decode_modeled_turn(turn: ModeledTurn) -> StepOutcome {
    // HEURISITC STRICLY FORBIDDEN.
    // Decode semantics may depend only on the current
    // provider turn and explicit protocol validity, never on ad hoc history-based
    // guesses such as "this looks like a repeated call".
    if !turn.has_tool_call {
        return StepOutcome::StructuralAnomaly;
    }

    if turn.multiple_tool_calls {
        return StepOutcome::StructuralAnomaly;
    }

    if !turn.arguments_valid {
        return StepOutcome::StructuralAnomaly;
    }

    if !turn.execution_successful {
        return StepOutcome::StructuralAnomaly;
    }

    modeled_outcome(turn.tool, turn.forced_terminal)
}

pub const fn is_progress(outcome: StepOutcome) -> bool {
    !matches!(outcome, StepOutcome::StructuralAnomaly)
}

pub const fn terminalize_after_non_progress(forced_turn: ModeledTurn) -> DriverOutcome {
    match decode_modeled_turn(forced_turn) {
        StepOutcome::Terminal => DriverOutcome::Answered,
        StepOutcome::NonTerminal | StepOutcome::StructuralAnomaly => DriverOutcome::Answered,
    }
}

pub const fn assistant_content_has_semantic_force(turn: ModeledTurn) -> bool {
    if turn.has_tool_call {
        return false;
    }

    false
}

pub const fn tool_class_from_discriminant(raw: u8) -> ToolClass {
    match raw % 12 {
        0 => ToolClass::Identity,
        1 => ToolClass::FollowUp,
        2 => ToolClass::SearchStack,
        3 => ToolClass::ExaSearch,
        4 => ToolClass::LightCompute,
        5 => ToolClass::ManageApp,
        6 => ToolClass::InspectApp,
        7 => ToolClass::ScratchThought,
        8 => ToolClass::ScratchpadSearch,
        9 => ToolClass::ScratchpadEdit,
        10 => ToolClass::StackProposal,
        _ => ToolClass::Unknown,
    }
}

pub const fn modeled_turn_from_discriminants(raw_tool: u8, raw_flags: u8) -> ModeledTurn {
    ModeledTurn {
        tool: tool_class_from_discriminant(raw_tool),
        has_tool_call: (raw_flags & 0b0000_0001) != 0,
        multiple_tool_calls: (raw_flags & 0b0100_0000) != 0,
        forced_terminal: (raw_flags & 0b0000_0010) != 0,
        assistant_content_present: (raw_flags & 0b0000_0100) != 0,
        arguments_valid: (raw_flags & 0b0000_1000) != 0,
        execution_successful: (raw_flags & 0b0010_0000) != 0,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        assistant_content_has_semantic_force, decode_modeled_turn, is_progress, modeled_outcome,
        terminalize_after_non_progress, DriverOutcome, ModeledTurn, StepOutcome,
        ToolClass,
    };

    #[test]
    fn only_identity_is_terminal_outside_forced_mode() {
        assert_eq!(modeled_outcome(ToolClass::Identity, false), StepOutcome::Terminal);
        assert_eq!(modeled_outcome(ToolClass::FollowUp, false), StepOutcome::NonTerminal);
        assert_eq!(modeled_outcome(ToolClass::StackProposal, false), StepOutcome::NonTerminal);
        assert_eq!(modeled_outcome(ToolClass::Unknown, false), StepOutcome::StructuralAnomaly);
    }

    #[test]
    fn forced_terminal_mode_accepts_only_identity() {
        assert_eq!(modeled_outcome(ToolClass::Identity, true), StepOutcome::Terminal);
        assert_eq!(modeled_outcome(ToolClass::FollowUp, true), StepOutcome::StructuralAnomaly);
        assert_eq!(modeled_outcome(ToolClass::ExaSearch, true), StepOutcome::StructuralAnomaly);
        assert_eq!(modeled_outcome(ToolClass::Unknown, true), StepOutcome::StructuralAnomaly);
    }

    #[test]
    fn bare_assistant_content_is_not_progress() {
        let turn = ModeledTurn {
            tool: ToolClass::Identity,
            has_tool_call: false,
            multiple_tool_calls: false,
            forced_terminal: false,
            assistant_content_present: true,
            arguments_valid: true,
            execution_successful: true,
        };

        assert_eq!(decode_modeled_turn(turn), StepOutcome::StructuralAnomaly);
        assert!(!is_progress(decode_modeled_turn(turn)));
        assert!(!assistant_content_has_semantic_force(turn));
    }

    #[test]
    fn malformed_arguments_are_not_progress() {
        let invalid_args = ModeledTurn {
            tool: ToolClass::SearchStack,
            has_tool_call: true,
            multiple_tool_calls: false,
            forced_terminal: false,
            assistant_content_present: false,
            arguments_valid: false,
            execution_successful: true,
        };

        assert_eq!(decode_modeled_turn(invalid_args), StepOutcome::StructuralAnomaly);
    }

    #[test]
    fn execution_failure_is_not_progress() {
        let turn = ModeledTurn {
            tool: ToolClass::LightCompute,
            has_tool_call: true,
            multiple_tool_calls: false,
            forced_terminal: false,
            assistant_content_present: true,
            arguments_valid: true,
            execution_successful: false,
        };

        assert_eq!(decode_modeled_turn(turn), StepOutcome::StructuralAnomaly);
    }

    #[test]
    fn assistant_narration_does_not_change_tool_semantics() {
        let with_content = ModeledTurn {
            tool: ToolClass::FollowUp,
            has_tool_call: true,
            multiple_tool_calls: false,
            forced_terminal: false,
            assistant_content_present: true,
            arguments_valid: true,
            execution_successful: true,
        };
        let without_content = ModeledTurn {
            assistant_content_present: false,
            ..with_content
        };

        assert_eq!(decode_modeled_turn(with_content), decode_modeled_turn(without_content));
        assert!(!assistant_content_has_semantic_force(with_content));
    }

    #[test]
    fn multiple_tool_calls_in_single_turn_are_not_progress() {
        let turn = ModeledTurn {
            tool: ToolClass::SearchStack,
            has_tool_call: true,
            multiple_tool_calls: true,
            forced_terminal: false,
            assistant_content_present: false,
            arguments_valid: true,
            execution_successful: true,
        };

        assert_eq!(decode_modeled_turn(turn), StepOutcome::StructuralAnomaly);
        assert!(!is_progress(decode_modeled_turn(turn)));
    }

    #[test]
    fn terminalization_after_non_progress_is_total() {
        let invalid_forced_turn = ModeledTurn {
            tool: ToolClass::FollowUp,
            has_tool_call: true,
            multiple_tool_calls: false,
            forced_terminal: true,
            assistant_content_present: false,
            arguments_valid: true,
            execution_successful: true,
        };

        assert_eq!(
            terminalize_after_non_progress(invalid_forced_turn),
            DriverOutcome::Answered
        );
    }
}

#[allow(unexpected_cfgs)]
#[cfg(kani)]
mod proofs {
    use super::{
        assistant_content_has_semantic_force, decode_modeled_turn, is_progress, modeled_outcome,
        modeled_turn_from_discriminants, terminalize_after_non_progress,
        tool_class_from_discriminant, DriverOutcome, ModeledTurn, StepOutcome, ToolClass,
    };

    #[kani::proof]
    fn forced_terminal_mode_never_accepts_non_identity() {
        let raw: u8 = kani::any();
        let tool = tool_class_from_discriminant(raw);

        if tool != ToolClass::Identity {
            assert!(modeled_outcome(tool, true) != StepOutcome::Terminal);
        }
    }

    #[kani::proof]
    fn follow_up_is_never_terminal() {
        assert!(modeled_outcome(ToolClass::FollowUp, false) == StepOutcome::NonTerminal);
        assert!(modeled_outcome(ToolClass::FollowUp, true) == StepOutcome::StructuralAnomaly);
    }

    #[kani::proof]
    fn identity_is_the_only_terminal_tool_class() {
        let raw: u8 = kani::any();
        let tool = tool_class_from_discriminant(raw);
        let outcome = modeled_outcome(tool, false);

        if outcome == StepOutcome::Terminal {
            assert!(tool == ToolClass::Identity);
        }
    }

    #[kani::proof]
    fn no_tool_call_never_counts_as_progress() {
        let raw_tool: u8 = kani::any();
        let raw_flags: u8 = kani::any();
        let mut turn = modeled_turn_from_discriminants(raw_tool, raw_flags);
        turn.has_tool_call = false;
        turn.multiple_tool_calls = false;

        let outcome = decode_modeled_turn(turn);
        assert!(outcome == StepOutcome::StructuralAnomaly);
        assert!(!is_progress(outcome));
    }

    #[kani::proof]
    fn invalid_arguments_never_produce_progress() {
        let raw_tool: u8 = kani::any();
        let raw_flags: u8 = kani::any();
        let mut turn = modeled_turn_from_discriminants(raw_tool, raw_flags);
        turn.has_tool_call = true;
        turn.multiple_tool_calls = false;
        turn.arguments_valid = false;

        assert!(decode_modeled_turn(turn) == StepOutcome::StructuralAnomaly);
    }

    #[kani::proof]
    fn execution_failure_never_produces_progress() {
        let raw_tool: u8 = kani::any();
        let raw_flags: u8 = kani::any();
        let mut turn = modeled_turn_from_discriminants(raw_tool, raw_flags);
        turn.has_tool_call = true;
        turn.multiple_tool_calls = false;
        turn.arguments_valid = true;
        turn.execution_successful = false;

        assert!(decode_modeled_turn(turn) == StepOutcome::StructuralAnomaly);
    }

    #[kani::proof]
    fn assistant_content_has_no_semantic_force() {
        let raw_tool: u8 = kani::any();
        let raw_flags: u8 = kani::any();
        let mut turn = modeled_turn_from_discriminants(raw_tool, raw_flags);
        turn.assistant_content_present = true;

        assert!(!assistant_content_has_semantic_force(turn));
    }

    #[kani::proof]
    fn terminal_progress_requires_identity_and_valid_decode() {
        let raw_tool: u8 = kani::any();
        let raw_flags: u8 = kani::any();
        let turn = modeled_turn_from_discriminants(raw_tool, raw_flags);
        let outcome = decode_modeled_turn(turn);

        if outcome == StepOutcome::Terminal {
            assert!(turn.has_tool_call);
            assert!(!turn.multiple_tool_calls);
            assert!(turn.arguments_valid);
            assert!(turn.execution_successful);
            assert!(turn.tool == ToolClass::Identity);
        }
    }

    #[kani::proof]
    fn multiple_tool_calls_in_single_turn_never_produce_progress() {
        let raw_tool: u8 = kani::any();
        let raw_flags: u8 = kani::any();
        let mut turn = modeled_turn_from_discriminants(raw_tool, raw_flags);
        turn.has_tool_call = true;
        turn.multiple_tool_calls = true;
        turn.arguments_valid = true;
        turn.execution_successful = true;

        let outcome = decode_modeled_turn(turn);
        assert!(outcome == StepOutcome::StructuralAnomaly);
        assert!(!is_progress(outcome));
    }

    #[kani::proof]
    fn assistant_content_does_not_change_decoded_outcome_when_tool_path_is_fixed() {
        let raw_tool: u8 = kani::any();
        let raw_flags: u8 = kani::any();
        let base = modeled_turn_from_discriminants(raw_tool, raw_flags);
        let with_content = ModeledTurn {
            assistant_content_present: true,
            ..base
        };
        let without_content = ModeledTurn {
            assistant_content_present: false,
            ..base
        };

        assert!(decode_modeled_turn(with_content) == decode_modeled_turn(without_content));
    }

    #[kani::proof]
    fn terminalization_after_non_progress_must_still_answer() {
        let raw_tool: u8 = kani::any();
        let raw_flags: u8 = kani::any();
        let mut forced_turn = modeled_turn_from_discriminants(raw_tool, raw_flags);
        forced_turn.has_tool_call = true;
        forced_turn.forced_terminal = true;

        assert!(terminalize_after_non_progress(forced_turn) == DriverOutcome::Answered);
    }
}