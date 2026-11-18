//! Command parser and execution engine for Tilo IPC and criteria rules.

use crate::criteria::{Criteria, parse_criteria_and_command};
use crate::focus::Direction;
use crate::layout::{LayoutType, SplitType};

/// Result of executing an individual command.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandResult {
    pub success: bool,
    pub error: Option<String>,
}

impl CommandResult {
    pub fn ok() -> Self {
        Self {
            success: true,
            error: None,
        }
    }

    pub fn err(msg: impl Into<String>) -> Self {
        Self {
            success: false,
            error: Some(msg.into()),
        }
    }
}

/// Splits a command chain separated by `;`, respecting single and double quotes.
pub fn split_command_chain(input: &str) -> Vec<String> {
    let mut commands = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut quote_char = '"';
    let mut escaped = false;

    for c in input.chars() {
        if escaped {
            current.push(c);
            escaped = false;
            continue;
        }
        if c == '\\' {
            current.push(c);
            escaped = true;
            continue;
        }
        if in_quotes {
            current.push(c);
            if c == quote_char {
                in_quotes = false;
            }
        } else if c == '"' || c == '\'' {
            in_quotes = true;
            quote_char = c;
            current.push(c);
        } else if c == ';' {
            let trimmed = current.trim();
            if !trimmed.is_empty() {
                commands.push(trimmed.to_string());
            }
            current.clear();
        } else {
            current.push(c);
        }
    }

    let trimmed = current.trim();
    if !trimmed.is_empty() {
        commands.push(trimmed.to_string());
    }

    commands
}

/// An executable command instruction.
#[derive(Debug, Clone, PartialEq)]
pub enum ParsedCommand {
    Split(SplitType),
    SplitToggle,
    Layout(LayoutType),
    LayoutToggle,
    FocusDirection(Direction),
    FocusParent,
    FocusChild,
    FocusNext,
    FocusPrev,
    FocusModeToggle,
    MoveDirection(Direction, Option<i32>),
    MovePosition { center: bool, x: Option<i32>, y: Option<i32> },
    MoveToWorkspace(String),
    MoveToOutput(String),
    MoveScratchpad,
    SwapWithId(u64),
    SwapWithMark(String),
    Fullscreen(Option<bool>),
    Floating(Option<bool>),
    Sticky(Option<bool>),
    Close,
    Kill,
    Workspace(String),
    WorkspaceNumber(i32),
    WorkspaceBackAndForth,
    MoveWorkspaceToOutput(String),
    Mark { name: String, add: bool, toggle: bool },
    Unmark(Option<String>),
    Border { mode: String, width: Option<u32> },
    Gaps { outer: bool, amount: i32 },
    Resize { dir: Direction, delta: i32 },
    Exec(String),
    Reload,
    Restart,
    Nop,
}

/// Parses a single command string into criteria and a `ParsedCommand`.
pub fn parse_command(input: &str) -> Result<(Option<Criteria>, ParsedCommand), String> {
    let (criteria, cmd_str) = parse_criteria_and_command(input)?;
    let parsed_cmd = parse_single_instruction(&cmd_str)?;
    Ok((criteria, parsed_cmd))
}

fn parse_single_instruction(cmd: &str) -> Result<ParsedCommand, String> {
    let tokens = tokenize_args(cmd);
    if tokens.is_empty() {
        return Ok(ParsedCommand::Nop);
    }

    match tokens[0].to_lowercase().as_str() {
        "split" | "splitv" | "splith" => {
            if tokens[0].eq_ignore_ascii_case("splitv") {
                return Ok(ParsedCommand::Split(SplitType::Vertical));
            }
            if tokens[0].eq_ignore_ascii_case("splith") {
                return Ok(ParsedCommand::Split(SplitType::Horizontal));
            }
            if tokens.len() < 2 {
                return Ok(ParsedCommand::SplitToggle);
            }
            match tokens[1].to_lowercase().as_str() {
                "v" | "vertical" => Ok(ParsedCommand::Split(SplitType::Vertical)),
                "h" | "horizontal" => Ok(ParsedCommand::Split(SplitType::Horizontal)),
                "t" | "toggle" => Ok(ParsedCommand::SplitToggle),
                other => Err(format!("Unknown split orientation: {}", other)),
            }
        }
        "layout" => {
            if tokens.len() < 2 {
                return Ok(ParsedCommand::LayoutToggle);
            }
            match tokens[1].to_lowercase().as_str() {
                "default" | "bsp" => Ok(ParsedCommand::Layout(LayoutType::Bsp)),
                "tabbed" => Ok(ParsedCommand::Layout(LayoutType::Tabbed)),
                "stacking" | "stacked" => Ok(ParsedCommand::Layout(LayoutType::Stacked)),
                "monocle" => Ok(ParsedCommand::Layout(LayoutType::Monocle)),
                "splitv" => Ok(ParsedCommand::Split(SplitType::Vertical)),
                "splith" => Ok(ParsedCommand::Split(SplitType::Horizontal)),
                "toggle" => Ok(ParsedCommand::LayoutToggle),
                other => Err(format!("Unknown layout: {}", other)),
            }
        }
        "focus" => {
            if tokens.len() < 2 {
                return Err("Missing focus target (e.g. left, right, parent, child)".to_string());
            }
            match tokens[1].to_lowercase().as_str() {
                "left" => Ok(ParsedCommand::FocusDirection(Direction::Left)),
                "right" => Ok(ParsedCommand::FocusDirection(Direction::Right)),
                "up" => Ok(ParsedCommand::FocusDirection(Direction::Up)),
                "down" => Ok(ParsedCommand::FocusDirection(Direction::Down)),
                "parent" => Ok(ParsedCommand::FocusParent),
                "child" => Ok(ParsedCommand::FocusChild),
                "next" => Ok(ParsedCommand::FocusNext),
                "prev" => Ok(ParsedCommand::FocusPrev),
                "tiling" | "floating" | "mode_toggle" => Ok(ParsedCommand::FocusModeToggle),
                "workspace" => {
                    if tokens.len() < 3 {
                        return Err("Missing workspace name in focus workspace".to_string());
                    }
                    Ok(ParsedCommand::Workspace(tokens[2].clone()))
                }
                other => Err(format!("Unknown focus direction: {}", other)),
            }
        }
        "workspace" => {
            if tokens.len() < 2 {
                return Err("Missing workspace name or action".to_string());
            }
            if tokens[1].eq_ignore_ascii_case("back_and_forth") {
                return Ok(ParsedCommand::WorkspaceBackAndForth);
            }
            if tokens[1].eq_ignore_ascii_case("number") {
                if tokens.len() < 3 {
                    return Err("Missing number in 'workspace number <n>'".to_string());
                }
                let num = tokens[2].parse::<i32>().map_err(|e| format!("Invalid workspace number: {}", e))?;
                return Ok(ParsedCommand::WorkspaceNumber(num));
            }
            Ok(ParsedCommand::Workspace(tokens[1].clone()))
        }
        "move" => {
            if tokens.len() < 2 {
                return Err("Missing move target".to_string());
            }
            let mut idx = 1;
            if tokens[idx].eq_ignore_ascii_case("container") || tokens[idx].eq_ignore_ascii_case("window") {
                idx += 1;
            }
            if idx < tokens.len() && tokens[idx].eq_ignore_ascii_case("to") {
                idx += 1;
            }
            if idx >= tokens.len() {
                return Err("Incomplete move command".to_string());
            }

            match tokens[idx].to_lowercase().as_str() {
                "workspace" => {
                    if idx + 1 >= tokens.len() {
                        return Err("Missing workspace name in move to workspace".to_string());
                    }
                    if tokens[idx + 1].eq_ignore_ascii_case("to") && idx + 2 < tokens.len() {
                        Ok(ParsedCommand::MoveToWorkspace(tokens[idx + 2].clone()))
                    } else {
                        Ok(ParsedCommand::MoveToWorkspace(tokens[idx + 1].clone()))
                    }
                }
                "output" => {
                    if idx + 1 >= tokens.len() {
                        return Err("Missing output name in move to output".to_string());
                    }
                    Ok(ParsedCommand::MoveToOutput(tokens[idx + 1].clone()))
                }
                "scratchpad" => Ok(ParsedCommand::MoveScratchpad),
                "position" | "absolute" => {
                    let is_center = tokens.iter().any(|t| t.eq_ignore_ascii_case("center"));
                    let nums: Vec<i32> = tokens.iter().filter_map(|t| t.parse::<i32>().ok()).collect();
                    if is_center {
                        Ok(ParsedCommand::MovePosition { center: true, x: None, y: None })
                    } else if nums.len() >= 2 {
                        Ok(ParsedCommand::MovePosition { center: false, x: Some(nums[0]), y: Some(nums[1]) })
                    } else {
                        Err("Invalid move position arguments".to_string())
                    }
                }
                "left" => Ok(ParsedCommand::MoveDirection(Direction::Left, tokens.get(idx + 1).and_then(|s| s.parse().ok()))),
                "right" => Ok(ParsedCommand::MoveDirection(Direction::Right, tokens.get(idx + 1).and_then(|s| s.parse().ok()))),
                "up" => Ok(ParsedCommand::MoveDirection(Direction::Up, tokens.get(idx + 1).and_then(|s| s.parse().ok()))),
                "down" => Ok(ParsedCommand::MoveDirection(Direction::Down, tokens.get(idx + 1).and_then(|s| s.parse().ok()))),
                other => Err(format!("Unknown move destination: {}", other)),
            }
        }
        "swap" => {
            // swap container with id <window-id> or swap container with mark <mark>
            let with_idx = tokens.iter().position(|t| t.eq_ignore_ascii_case("with"));
            if let Some(pos) = with_idx {
                if pos + 2 < tokens.len() {
                    let target_type = &tokens[pos + 1];
                    let target_val = &tokens[pos + 2];
                    if target_type.eq_ignore_ascii_case("mark") {
                        return Ok(ParsedCommand::SwapWithMark(target_val.clone()));
                    } else if target_type.eq_ignore_ascii_case("id") {
                        let id = if target_val.starts_with("0x") || target_val.starts_with("0X") {
                            u64::from_str_radix(&target_val[2..], 16)
                        } else {
                            target_val.parse::<u64>()
                        }.map_err(|e| format!("Invalid swap id: {}", e))?;
                        return Ok(ParsedCommand::SwapWithId(id));
                    }
                }
            }
            Err("Expected 'swap [container] with [mark|id] <value>'".to_string())
        }
        "mark" => {
            let mut add = false;
            let mut toggle = false;
            let mut name = String::new();

            for arg in &tokens[1..] {
                if arg == "--add" {
                    add = true;
                } else if arg == "--toggle" {
                    toggle = true;
                } else if arg == "--replace" {
                    // default
                } else if name.is_empty() {
                    name = arg.clone();
                }
            }

            if name.is_empty() {
                return Err("Missing mark name".to_string());
            }
            Ok(ParsedCommand::Mark { name, add, toggle })
        }
        "unmark" => {
            let name = tokens.get(1).cloned();
            Ok(ParsedCommand::Unmark(name))
        }
        "fullscreen" => {
            if tokens.len() < 2 || tokens[1].eq_ignore_ascii_case("toggle") {
                Ok(ParsedCommand::Fullscreen(None))
            } else if tokens[1].eq_ignore_ascii_case("enable") || tokens[1].eq_ignore_ascii_case("1") {
                Ok(ParsedCommand::Fullscreen(Some(true)))
            } else {
                Ok(ParsedCommand::Fullscreen(Some(false)))
            }
        }
        "floating" => {
            if tokens.len() < 2 || tokens[1].eq_ignore_ascii_case("toggle") {
                Ok(ParsedCommand::Floating(None))
            } else if tokens[1].eq_ignore_ascii_case("enable") || tokens[1].eq_ignore_ascii_case("1") {
                Ok(ParsedCommand::Floating(Some(true)))
            } else {
                Ok(ParsedCommand::Floating(Some(false)))
            }
        }
        "sticky" => {
            if tokens.len() < 2 || tokens[1].eq_ignore_ascii_case("toggle") {
                Ok(ParsedCommand::Sticky(None))
            } else if tokens[1].eq_ignore_ascii_case("enable") {
                Ok(ParsedCommand::Sticky(Some(true)))
            } else {
                Ok(ParsedCommand::Sticky(Some(false)))
            }
        }
        "close" => Ok(ParsedCommand::Close),
        "kill" => Ok(ParsedCommand::Kill),
        "border" => {
            let mode = tokens.get(1).cloned().unwrap_or_else(|| "toggle".to_string());
            let width = tokens.get(2).and_then(|w| w.parse().ok());
            Ok(ParsedCommand::Border { mode, width })
        }
        "gaps" => {
            let outer = tokens.iter().any(|t| t.eq_ignore_ascii_case("outer"));
            let amount = tokens.iter().filter_map(|t| t.parse::<i32>().ok()).next().unwrap_or(4);
            Ok(ParsedCommand::Gaps { outer, amount })
        }
        "resize" => {
            let is_shrink = tokens.iter().any(|t| t.eq_ignore_ascii_case("shrink"));
            let amount = tokens.iter().filter_map(|t| t.parse::<i32>().ok()).next().unwrap_or(20);
            let delta = if is_shrink { -amount } else { amount };
            let dir = if tokens.iter().any(|t| t.eq_ignore_ascii_case("height") || t.eq_ignore_ascii_case("up") || t.eq_ignore_ascii_case("down")) {
                Direction::Down
            } else {
                Direction::Right
            };
            Ok(ParsedCommand::Resize { dir, delta })
        }
        "exec" | "exec_already" => {
            let cmd_to_exec = tokens[1..].join(" ");
            Ok(ParsedCommand::Exec(cmd_to_exec))
        }
        "reload" => Ok(ParsedCommand::Reload),
        "restart" => Ok(ParsedCommand::Restart),
        "nop" => Ok(ParsedCommand::Nop),
        other => Err(format!("Unknown command: {}", other)),
    }
}

fn tokenize_args(input: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut quote_char = '"';
    let mut escaped = false;

    for c in input.chars() {
        if escaped {
            current.push(c);
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
            continue;
        }
        if in_quotes {
            if c == quote_char {
                in_quotes = false;
            } else {
                current.push(c);
            }
        } else if c == '"' || c == '\'' {
            in_quotes = true;
            quote_char = c;
        } else if c.is_whitespace() {
            if !current.is_empty() {
                args.push(current);
                current = String::new();
            }
        } else {
            current.push(c);
        }
    }

    if !current.is_empty() {
        args.push(current);
    }

    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_command_chain() {
        let chain = "split v; layout tabbed; exec \"echo 'hello; world'\"";
        let parts = split_command_chain(chain);
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0], "split v");
        assert_eq!(parts[1], "layout tabbed");
        assert_eq!(parts[2], "exec \"echo 'hello; world'\"");
    }

    #[test]
    fn test_parse_commands() {
        let (_, cmd) = parse_command("workspace \"2:code\"").expect("parse ok");
        assert_eq!(cmd, ParsedCommand::Workspace("2:code".to_string()));

        let (_, cmd) = parse_command("mark --add browser").expect("parse ok");
        assert_eq!(cmd, ParsedCommand::Mark { name: "browser".to_string(), add: true, toggle: false });

        let (_, cmd) = parse_command("swap container with mark editor").expect("parse ok");
        assert_eq!(cmd, ParsedCommand::SwapWithMark("editor".to_string()));

        let (crit, cmd) = parse_command("[class=\"Firefox\"] floating enable").expect("parse ok");
        assert!(crit.is_some());
        assert_eq!(cmd, ParsedCommand::Floating(Some(true)));
    }
}
