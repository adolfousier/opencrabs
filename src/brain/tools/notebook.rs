//! Jupyter Notebook Edit Tool
//!
//! Modify Jupyter notebook files (.ipynb) cell by cell.

use super::error::{Result, ToolError};
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use tokio::fs;

/// Jupyter notebook edit tool
pub struct NotebookEditTool;

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "operation")]
enum NotebookOperation {
    /// Add a new cell
    #[serde(rename = "add_cell")]
    AddCell {
        cell_type: String,
        source: Vec<String>,
        /// Position to insert (if None, append to end)
        #[serde(skip_serializing_if = "Option::is_none")]
        position: Option<usize>,
    },

    /// Edit an existing cell
    #[serde(rename = "edit_cell")]
    EditCell {
        /// Cell index (0-based)
        index: usize,
        /// New cell source
        source: Vec<String>,
    },

    /// Delete a cell
    #[serde(rename = "delete_cell")]
    DeleteCell {
        /// Cell index (0-based)
        index: usize,
    },

    /// Clear all cell outputs
    #[serde(rename = "clear_outputs")]
    ClearOutputs,
}

#[derive(Debug, Deserialize, Serialize)]
struct NotebookInput {
    /// Path to notebook file
    path: String,

    /// Operation to perform
    #[serde(flatten)]
    operation: NotebookOperation,

    /// Create backup before editing
    #[serde(default = "default_true")]
    create_backup: bool,
}

fn default_true() -> bool {
    true
}

// Simplified Jupyter notebook structure
#[derive(Debug, Deserialize, Serialize)]
struct Notebook {
    cells: Vec<Cell>,
    metadata: Value,
    nbformat: i32,
    nbformat_minor: i32,
}

#[derive(Debug, Deserialize, Serialize)]
struct Cell {
    cell_type: String,
    source: Vec<String>,
    metadata: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    outputs: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    execution_count: Option<Value>,
}

#[async_trait]
impl Tool for NotebookEditTool {
    fn name(&self) -> &str {
        "notebook_edit"
    }

    fn description(&self) -> &str {
        "Edit Jupyter notebook files (.ipynb) cell by cell. Supports adding, editing, deleting cells, and clearing outputs."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the .ipynb notebook file"
                },
                "operation": {
                    "type": "string",
                    "description": "Operation to perform",
                    "enum": ["add_cell", "edit_cell", "delete_cell", "clear_outputs"]
                },
                "cell_type": {
                    "type": "string",
                    "description": "Cell type for add_cell operation",
                    "enum": ["code", "markdown", "raw"]
                },
                "source": {
                    "type": "array",
                    "description": "Cell source code/text as array of strings",
                    "items": {
                        "type": "string"
                    }
                },
                "position": {
                    "type": "integer",
                    "description": "Position to insert cell (0-based, for add_cell)",
                    "minimum": 0
                },
                "index": {
                    "type": "integer",
                    "description": "Cell index for edit_cell or delete_cell (0-based)",
                    "minimum": 0
                },
                "create_backup": {
                    "type": "boolean",
                    "description": "Create backup before editing (default: true)",
                    "default": true
                }
            },
            "required": ["path", "operation"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![
            ToolCapability::ReadFiles,
            ToolCapability::WriteFiles,
            ToolCapability::SystemModification,
        ]
    }

    fn requires_approval(&self) -> bool {
        true // Modifying notebooks requires approval
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        let _: NotebookInput = serde_json::from_value(input.clone())
            .map_err(|e| ToolError::InvalidInput(format!("Invalid input: {}", e)))?;
        Ok(())
    }

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let input: NotebookInput = serde_json::from_value(input)?;

        // Resolve path
        let path = if PathBuf::from(&input.path).is_absolute() {
            PathBuf::from(&input.path)
        } else {
            context.working_directory.join(&input.path)
        };

        // Check if file exists and is a notebook
        if !path.exists() {
            return Ok(ToolResult::error(format!(
                "Notebook file not found: {}",
                path.display()
            )));
        }

        if path.extension().and_then(|s| s.to_str()) != Some("ipynb") {
            return Ok(ToolResult::error(
                "File must have .ipynb extension".to_string(),
            ));
        }

        // Read notebook
        let content = fs::read_to_string(&path).await.map_err(ToolError::Io)?;
        let mut notebook: Notebook = serde_json::from_str(&content)
            .map_err(|e| ToolError::InvalidInput(format!("Invalid notebook format: {}", e)))?;

        // Create backup if requested
        if input.create_backup {
            let backup_path = path.with_extension("ipynb.backup");
            fs::write(&backup_path, &content)
                .await
                .map_err(ToolError::Io)?;
        }

        // Perform operation
        let result_message = match input.operation {
            NotebookOperation::AddCell {
                cell_type,
                source,
                position,
            } => {
                // Validate cell type
                if !["code", "markdown", "raw"].contains(&cell_type.as_str()) {
                    return Ok(ToolResult::error(format!(
                        "Invalid cell type: {}. Must be 'code', 'markdown', or 'raw'",
                        cell_type
                    )));
                }

                let new_cell = Cell {
                    cell_type: cell_type.clone(),
                    source,
                    metadata: serde_json::json!({}),
                    outputs: if cell_type == "code" {
                        Some(vec![])
                    } else {
                        None
                    },
                    execution_count: if cell_type == "code" {
                        Some(Value::Null)
                    } else {
                        None
                    },
                };

                if let Some(pos) = position {
                    if pos > notebook.cells.len() {
                        return Ok(ToolResult::error(format!(
                            "Position {} out of bounds (notebook has {} cells)",
                            pos,
                            notebook.cells.len()
                        )));
                    }
                    notebook.cells.insert(pos, new_cell);
                    format!("Added {} cell at position {}", cell_type, pos)
                } else {
                    notebook.cells.push(new_cell);
                    format!(
                        "Added {} cell at end (position {})",
                        cell_type,
                        notebook.cells.len() - 1
                    )
                }
            }

            NotebookOperation::EditCell { index, source } => {
                if index >= notebook.cells.len() {
                    return Ok(ToolResult::error(format!(
                        "Cell index {} out of bounds (notebook has {} cells)",
                        index,
                        notebook.cells.len()
                    )));
                }

                notebook.cells[index].source = source;
                // Clear outputs when editing code cells
                if notebook.cells[index].cell_type == "code" {
                    notebook.cells[index].outputs = Some(vec![]);
                    notebook.cells[index].execution_count = Some(Value::Null);
                }

                format!(
                    "Edited cell {} ({})",
                    index, notebook.cells[index].cell_type
                )
            }

            NotebookOperation::DeleteCell { index } => {
                if index >= notebook.cells.len() {
                    return Ok(ToolResult::error(format!(
                        "Cell index {} out of bounds (notebook has {} cells)",
                        index,
                        notebook.cells.len()
                    )));
                }

                let removed_cell = notebook.cells.remove(index);
                format!(
                    "Deleted cell {} ({}, {} cells remaining)",
                    index,
                    removed_cell.cell_type,
                    notebook.cells.len()
                )
            }

            NotebookOperation::ClearOutputs => {
                let mut cleared_count = 0;
                for cell in &mut notebook.cells {
                    if cell.cell_type == "code" {
                        cell.outputs = Some(vec![]);
                        cell.execution_count = Some(Value::Null);
                        cleared_count += 1;
                    }
                }
                format!("Cleared outputs from {} code cells", cleared_count)
            }
        };

        // Write modified notebook
        let new_content = serde_json::to_string_pretty(&notebook)
            .map_err(|e| ToolError::Execution(format!("Failed to serialize notebook: {}", e)))?;

        fs::write(&path, new_content).await.map_err(ToolError::Io)?;

        Ok(ToolResult::success(format!(
            "{}. Notebook saved: {}",
            result_message,
            path.display()
        )))
    }
}
