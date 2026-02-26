use chrono::Local;
use crossterm::event::{KeyCode, KeyEvent};

use super::types::*;
use super::wizard::OnboardingWizard;

impl OnboardingWizard {
    pub(super) fn handle_brain_setup_key(&mut self, event: KeyEvent) -> WizardAction {
        // Don't accept input while generating
        if self.brain_generating {
            return WizardAction::None;
        }

        // If already generated or errored, Enter advances
        if self.brain_generated || self.brain_error.is_some() {
            if event.code == KeyCode::Enter {
                self.next_step();
                return WizardAction::Complete;
            }
            return WizardAction::None;
        }

        match event.code {
            KeyCode::Esc => {
                // Esc always skips
                self.step = OnboardingStep::Complete;
                return WizardAction::Complete;
            }
            KeyCode::Tab => {
                self.brain_field = match self.brain_field {
                    BrainField::AboutMe => BrainField::AboutAgent,
                    BrainField::AboutAgent => BrainField::AboutMe,
                };
            }
            KeyCode::BackTab => {
                self.brain_field = match self.brain_field {
                    BrainField::AboutMe => BrainField::AboutAgent,
                    BrainField::AboutAgent => BrainField::AboutMe,
                };
            }
            KeyCode::Enter => {
                if self.brain_field == BrainField::AboutAgent {
                    if self.about_me.is_empty() && self.about_opencrabs.is_empty() {
                        // Nothing to work with — skip straight to Complete
                        self.step = OnboardingStep::Complete;
                        return WizardAction::Complete;
                    }
                    // If inputs unchanged from loaded values, skip without regenerating
                    if !self.brain_inputs_changed() && !self.original_about_me.is_empty() {
                        self.step = OnboardingStep::Complete;
                        return WizardAction::Complete;
                    }
                    // Inputs changed or new — trigger generation
                    return WizardAction::GenerateBrain;
                }
                // Enter on AboutMe moves to AboutAgent
                self.brain_field = BrainField::AboutAgent;
            }
            KeyCode::Char(c) => {
                self.active_brain_field_mut().push(c);
            }
            KeyCode::Backspace => {
                self.active_brain_field_mut().pop();
            }
            _ => {}
        }
        WizardAction::None
    }

    /// Get mutable reference to the currently focused brain text area
    fn active_brain_field_mut(&mut self) -> &mut String {
        match self.brain_field {
            BrainField::AboutMe => &mut self.about_me,
            BrainField::AboutAgent => &mut self.about_opencrabs,
        }
    }

    /// Whether brain inputs have been modified since loading from file
    fn brain_inputs_changed(&self) -> bool {
        self.about_me != self.original_about_me
            || self.about_opencrabs != self.original_about_opencrabs
    }

    /// Truncate file content to first N chars for preview in the wizard
    pub(super) fn truncate_preview(content: &str, max_chars: usize) -> String {
        let trimmed = content.trim();
        if trimmed.len() <= max_chars {
            trimmed.to_string()
        } else {
            let truncated = &trimmed[..trimmed.floor_char_boundary(max_chars)];
            format!("{}...", truncated.trim_end())
        }
    }

    /// Build the prompt sent to the AI to generate personalized brain files.
    /// Uses existing workspace files if available, falls back to static templates.
    pub fn build_brain_prompt(&self) -> String {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let workspace = std::path::Path::new(&self.workspace_path);

        // Read current brain files from workspace, fall back to static templates
        let soul_template_static = include_str!("../../docs/reference/templates/SOUL.md");
        let identity_template_static = include_str!("../../docs/reference/templates/IDENTITY.md");
        let user_template_static = include_str!("../../docs/reference/templates/USER.md");
        let agents_template_static = include_str!("../../docs/reference/templates/AGENTS.md");
        let tools_template_static = include_str!("../../docs/reference/templates/TOOLS.md");
        let memory_template_static = include_str!("../../docs/reference/templates/MEMORY.md");

        let soul_template = std::fs::read_to_string(workspace.join("SOUL.md"))
            .unwrap_or_else(|_| soul_template_static.to_string());
        let identity_template = std::fs::read_to_string(workspace.join("IDENTITY.md"))
            .unwrap_or_else(|_| identity_template_static.to_string());
        let user_template = std::fs::read_to_string(workspace.join("USER.md"))
            .unwrap_or_else(|_| user_template_static.to_string());
        let agents_template = std::fs::read_to_string(workspace.join("AGENTS.md"))
            .unwrap_or_else(|_| agents_template_static.to_string());
        let tools_template = std::fs::read_to_string(workspace.join("TOOLS.md"))
            .unwrap_or_else(|_| tools_template_static.to_string());
        let memory_template = std::fs::read_to_string(workspace.join("MEMORY.md"))
            .unwrap_or_else(|_| memory_template_static.to_string());

        format!(
            r#"You are setting up a personal AI agent's brain — its entire workspace of markdown files that define who it is, who its human is, and how it operates.

The user dumped two blocks of info. One about themselves (name, role, links, projects, whatever they shared). One about how they want their agent to be (personality, vibe, behavior). Use EVERYTHING they gave you to personalize ALL six template files below.

=== ABOUT THE USER ===
{about_me}

=== ABOUT THE AGENT ===
{about_opencrabs}

=== TODAY'S DATE ===
{date}

Below are the 6 template files. Replace ALL <placeholder> tags and HTML comments with real values based on what the user provided. Keep the exact markdown structure. Fill what you can from the user's info, leave sensible defaults for anything not provided. Don't invent facts — if the user didn't mention something, use a reasonable placeholder like "TBD" or remove that line.

===TEMPLATE: SOUL.md===
{soul}

===TEMPLATE: IDENTITY.md===
{identity}

===TEMPLATE: USER.md===
{user}

===TEMPLATE: AGENTS.md===
{agents}

===TEMPLATE: TOOLS.md===
{tools}

===TEMPLATE: MEMORY.md===
{memory}

Respond with EXACTLY six sections using these delimiters. No extra text before the first delimiter or after the last section:
---SOUL---
(generated SOUL.md content)
---IDENTITY---
(generated IDENTITY.md content)
---USER---
(generated USER.md content)
---AGENTS---
(generated AGENTS.md content)
---TOOLS---
(generated TOOLS.md content)
---MEMORY---
(generated MEMORY.md content)"#,
            about_me = if self.about_me.is_empty() {
                "Not provided"
            } else {
                &self.about_me
            },
            about_opencrabs = if self.about_opencrabs.is_empty() {
                "Not provided"
            } else {
                &self.about_opencrabs
            },
            date = today,
            soul = soul_template,
            identity = identity_template,
            user = user_template,
            agents = agents_template,
            tools = tools_template,
            memory = memory_template,
        )
    }

    /// Store the generated brain content from the AI response
    pub fn apply_generated_brain(&mut self, response: &str) {
        // Parse the response into six sections using delimiters
        let delimiters = [
            "---SOUL---",
            "---IDENTITY---",
            "---USER---",
            "---AGENTS---",
            "---TOOLS---",
            "---MEMORY---",
        ];

        // Find all delimiter positions
        let positions: Vec<Option<usize>> = delimiters.iter().map(|d| response.find(d)).collect();

        // Need at least SOUL, IDENTITY, USER to consider it a success
        if positions[0].is_none() || positions[1].is_none() || positions[2].is_none() {
            self.brain_error = Some("Couldn't parse AI response — using defaults".to_string());
            self.brain_generating = false;
            return;
        }

        // Extract content between delimiters
        // Build ordered list of (delimiter_index, position) sorted by position
        let mut ordered: Vec<(usize, usize)> = positions
            .iter()
            .enumerate()
            .filter_map(|(i, pos)| pos.map(|p| (i, p)))
            .collect();
        ordered.sort_by_key(|(_, pos)| *pos);

        for (idx, &(delim_idx, pos)) in ordered.iter().enumerate() {
            let start = pos + delimiters[delim_idx].len();
            let end = if idx + 1 < ordered.len() {
                ordered[idx + 1].1
            } else {
                response.len()
            };
            let content = response[start..end].trim();

            if !content.is_empty() {
                match delim_idx {
                    0 => self.generated_soul = Some(content.to_string()),
                    1 => self.generated_identity = Some(content.to_string()),
                    2 => self.generated_user = Some(content.to_string()),
                    3 => self.generated_agents = Some(content.to_string()),
                    4 => self.generated_tools = Some(content.to_string()),
                    5 => self.generated_memory = Some(content.to_string()),
                    _ => {}
                }
            }
        }

        self.brain_generated = true;
        self.brain_generating = false;
    }
}
