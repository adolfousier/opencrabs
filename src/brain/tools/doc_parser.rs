//! Document Parser Tool
//!
//! Parses various document formats (PDF, DOCX, TXT, etc.) to extract text content.

use super::error::{Result, ToolError};
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Document Parser Tool - extracts text from various document formats
pub struct DocParserTool;

/// Maximum file size for document parsing (50MB)
/// This prevents memory exhaustion from very large documents
const MAX_FILE_SIZE: u64 = 50 * 1024 * 1024;

#[derive(Debug, Deserialize, Serialize)]
struct DocParserInput {
    /// Path to the document file
    path: String,

    /// Optional: Maximum characters to extract (default: no limit)
    #[serde(skip_serializing_if = "Option::is_none")]
    max_chars: Option<usize>,

    /// Optional: Specific pages to extract (PDF only, 1-indexed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pages: Option<Vec<usize>>,

    /// Optional: Include metadata in output
    #[serde(skip_serializing_if = "Option::is_none")]
    include_metadata: Option<bool>,
}

#[derive(Debug, Serialize)]
struct DocumentMetadata {
    format: String,
    file_size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    page_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    author: Option<String>,
}

#[async_trait]
impl Tool for DocParserTool {
    fn name(&self) -> &str {
        "parse_document"
    }

    fn description(&self) -> &str {
        "Parse and extract text content from documents (PDF, DOCX, TXT, MD, HTML). \
        Useful for analyzing documents, extracting information, and converting document content to plain text."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the document file (PDF, DOCX, TXT, MD, HTML)"
                },
                "max_chars": {
                    "type": "integer",
                    "description": "Optional: Maximum characters to extract (default: unlimited)",
                    "minimum": 1
                },
                "pages": {
                    "type": "array",
                    "items": {"type": "integer", "minimum": 1},
                    "description": "Optional: Specific page numbers to extract (PDF only, 1-indexed)"
                },
                "include_metadata": {
                    "type": "boolean",
                    "description": "Optional: Include document metadata in output (default: false)"
                }
            },
            "required": ["path"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::ReadFiles]
    }

    fn requires_approval(&self) -> bool {
        false // Reading documents is generally safe
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        let _: DocParserInput = serde_json::from_value(input.clone())
            .map_err(|e| ToolError::InvalidInput(format!("Invalid input: {}", e)))?;
        Ok(())
    }

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let input: DocParserInput = serde_json::from_value(input)?;

        // Resolve path relative to working directory
        let path = if PathBuf::from(&input.path).is_absolute() {
            PathBuf::from(&input.path)
        } else {
            context.working_directory.join(&input.path)
        };

        // Check if file exists
        if !path.exists() {
            return Ok(ToolResult::error(format!(
                "File not found: {}",
                path.display()
            )));
        }

        // Check if it's a file (not a directory)
        if !path.is_file() {
            return Ok(ToolResult::error(format!(
                "Path is not a file: {}",
                path.display()
            )));
        }

        // Check file size to prevent memory exhaustion
        let file_size = std::fs::metadata(&path).map_err(ToolError::Io)?.len();
        if file_size > MAX_FILE_SIZE {
            return Ok(ToolResult::error(format!(
                "File size ({} MB) exceeds maximum allowed size ({} MB). \
                Consider splitting the document or using a different approach.",
                file_size / (1024 * 1024),
                MAX_FILE_SIZE / (1024 * 1024)
            )));
        }

        // Determine file type and parse accordingly
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();

        let (text, metadata) = match extension.as_str() {
            "pdf" => self.parse_pdf(&path, &input).await?,
            "docx" => self.parse_docx(&path).await?,
            "txt" | "md" | "markdown" | "rst" | "text" => {
                self.parse_text(&path, &extension).await?
            }
            "html" | "htm" => self.parse_html(&path).await?,
            "json" => self.parse_json(&path).await?,
            "xml" => self.parse_xml(&path).await?,
            _ => {
                return Ok(ToolResult::error(format!(
                    "Unsupported document format: .{}. Supported formats: PDF, DOCX, TXT, MD, HTML, JSON, XML",
                    extension
                )));
            }
        };

        // Apply character limit if specified
        let text = if let Some(max_chars) = input.max_chars {
            if text.len() > max_chars {
                format!(
                    "{}...\n\n[Truncated: {} of {} characters shown]",
                    crate::utils::truncate_str(&text, max_chars),
                    max_chars,
                    text.len()
                )
            } else {
                text
            }
        } else {
            text
        };

        // Build output
        let output = if input.include_metadata.unwrap_or(false) {
            let meta = DocumentMetadata {
                format: extension.clone(),
                file_size,
                page_count: metadata.page_count,
                title: metadata.title,
                author: metadata.author,
            };
            format!(
                "=== Document Metadata ===\n{}\n\n=== Content ===\n{}",
                serde_json::to_string_pretty(&meta).unwrap_or_default(),
                text
            )
        } else {
            text
        };

        let output_len = output.len();
        Ok(ToolResult::success(output)
            .with_metadata("path".to_string(), path.display().to_string())
            .with_metadata("format".to_string(), extension)
            .with_metadata("chars".to_string(), output_len.to_string()))
    }
}

#[derive(Default)]
struct ParsedMetadata {
    page_count: Option<usize>,
    title: Option<String>,
    author: Option<String>,
}

impl DocParserTool {
    /// Parse PDF document
    async fn parse_pdf(
        &self,
        path: &Path,
        input: &DocParserInput,
    ) -> Result<(String, ParsedMetadata)> {
        let path = path.to_path_buf();
        let pages = input.pages.clone();

        // Run PDF parsing in blocking task
        tokio::task::spawn_blocking(move || {
            let bytes = std::fs::read(&path).map_err(ToolError::Io)?;

            // Extract text from PDF
            let text = pdf_extract::extract_text_from_mem(&bytes)
                .map_err(|e| ToolError::Execution(format!("Failed to parse PDF: {}", e)))?;

            // If specific pages requested, filter them
            let text = if let Some(page_nums) = pages {
                // Split by page breaks (form feed character or multiple newlines)
                let pages: Vec<&str> = text.split("\u{000C}").collect();
                let mut selected_text = String::new();

                for page_num in page_nums {
                    if page_num > 0 && page_num <= pages.len() {
                        selected_text.push_str(&format!("--- Page {} ---\n", page_num));
                        selected_text.push_str(pages[page_num - 1].trim());
                        selected_text.push_str("\n\n");
                    }
                }

                if selected_text.is_empty() {
                    text // Fall back to full text if no valid pages
                } else {
                    selected_text
                }
            } else {
                text
            };

            // Count pages (rough estimate based on form feeds)
            let page_count = text.matches("\u{000C}").count() + 1;

            let metadata = ParsedMetadata {
                page_count: Some(page_count),
                title: None,
                author: None,
            };

            Ok((text.trim().to_string(), metadata))
        })
        .await
        .map_err(|e| ToolError::Execution(format!("PDF parsing task failed: {}", e)))?
    }

    /// Parse DOCX document (Office Open XML)
    async fn parse_docx(&self, path: &Path) -> Result<(String, ParsedMetadata)> {
        let path = path.to_path_buf();

        tokio::task::spawn_blocking(move || {
            let file = std::fs::File::open(&path).map_err(ToolError::Io)?;
            let mut archive = zip::ZipArchive::new(file)
                .map_err(|e| ToolError::Execution(format!("Failed to open DOCX: {}", e)))?;

            let mut text_content = String::new();
            let mut title = None;
            let mut author = None;

            // Extract document.xml (main content)
            if let Ok(mut document_xml) = archive.by_name("word/document.xml") {
                let mut xml_content = String::new();
                document_xml
                    .read_to_string(&mut xml_content)
                    .map_err(ToolError::Io)?;

                // Parse XML to extract text
                text_content = Self::extract_text_from_docx_xml(&xml_content);
            }

            // Extract core.xml (metadata)
            if let Ok(mut core_xml) = archive.by_name("docProps/core.xml") {
                let mut xml_content = String::new();
                core_xml
                    .read_to_string(&mut xml_content)
                    .map_err(ToolError::Io)?;

                let (t, a) = Self::extract_metadata_from_core_xml(&xml_content);
                title = t;
                author = a;
            }

            let metadata = ParsedMetadata {
                page_count: None, // DOCX doesn't store page count in metadata
                title,
                author,
            };

            Ok((text_content, metadata))
        })
        .await
        .map_err(|e| ToolError::Execution(format!("DOCX parsing task failed: {}", e)))?
    }

    /// Extract text from DOCX XML content
    fn extract_text_from_docx_xml(xml: &str) -> String {
        let mut text = String::new();
        let mut reader = quick_xml::Reader::from_str(xml);
        let mut buf = Vec::new();
        let mut in_text = false;

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(quick_xml::events::Event::Start(ref e)) => {
                    // w:t is the text element in DOCX
                    if e.name().as_ref() == b"w:t" {
                        in_text = true;
                    }
                    // w:p is paragraph - add newline after each
                    if e.name().as_ref() == b"w:p" && !text.is_empty() {
                        text.push('\n');
                    }
                }
                Ok(quick_xml::events::Event::Text(e)) => {
                    if in_text && let Ok(t) = e.unescape() {
                        text.push_str(&t);
                    }
                }
                Ok(quick_xml::events::Event::End(ref e)) => {
                    if e.name().as_ref() == b"w:t" {
                        in_text = false;
                    }
                }
                Ok(quick_xml::events::Event::Eof) => break,
                Err(_) => break,
                _ => {}
            }
            buf.clear();
        }

        text.trim().to_string()
    }

    /// Extract metadata from DOCX core.xml
    fn extract_metadata_from_core_xml(xml: &str) -> (Option<String>, Option<String>) {
        let mut title = None;
        let mut author = None;
        let mut reader = quick_xml::Reader::from_str(xml);
        let mut buf = Vec::new();
        let mut current_element = String::new();

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(quick_xml::events::Event::Start(ref e)) => {
                    current_element = String::from_utf8_lossy(e.name().as_ref()).to_string();
                }
                Ok(quick_xml::events::Event::Text(e)) => {
                    if let Ok(t) = e.unescape() {
                        match current_element.as_str() {
                            "dc:title" => title = Some(t.to_string()),
                            "dc:creator" => author = Some(t.to_string()),
                            _ => {}
                        }
                    }
                }
                Ok(quick_xml::events::Event::Eof) => break,
                Err(_) => break,
                _ => {}
            }
            buf.clear();
        }

        (title, author)
    }

    /// Parse plain text files
    async fn parse_text(&self, path: &Path, _format: &str) -> Result<(String, ParsedMetadata)> {
        let text = tokio::fs::read_to_string(path)
            .await
            .map_err(ToolError::Io)?;

        let metadata = ParsedMetadata {
            page_count: None,
            title: None,
            author: None,
        };

        Ok((text, metadata))
    }

    /// Parse HTML files
    async fn parse_html(&self, path: &Path) -> Result<(String, ParsedMetadata)> {
        let html = tokio::fs::read_to_string(path)
            .await
            .map_err(ToolError::Io)?;

        // Simple HTML tag removal
        let text = Self::strip_html_tags(&html);

        let metadata = ParsedMetadata {
            page_count: None,
            title: Self::extract_html_title(&html),
            author: None,
        };

        Ok((text, metadata))
    }

    /// Strip HTML tags from content
    fn strip_html_tags(html: &str) -> String {
        let mut text = String::new();
        let mut in_tag = false;
        let mut in_script = false;
        let mut in_style = false;

        let chars: Vec<char> = html.chars().collect();
        let mut i = 0;

        while i < chars.len() {
            let c = chars[i];

            if c == '<' {
                in_tag = true;
                // Check for script/style tags
                let remaining: String = chars[i..].iter().take(10).collect();
                if remaining.to_lowercase().starts_with("<script") {
                    in_script = true;
                } else if remaining.to_lowercase().starts_with("<style") {
                    in_style = true;
                } else if remaining.to_lowercase().starts_with("</script") {
                    in_script = false;
                } else if remaining.to_lowercase().starts_with("</style") {
                    in_style = false;
                }
            } else if c == '>' {
                in_tag = false;
            } else if !in_tag && !in_script && !in_style {
                text.push(c);
            }

            i += 1;
        }

        // Clean up whitespace
        let lines: Vec<&str> = text
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect();
        lines.join("\n")
    }

    /// Extract title from HTML
    fn extract_html_title(html: &str) -> Option<String> {
        let lowercase = html.to_lowercase();
        if let Some(start) = lowercase.find("<title>") {
            let start = start + 7;
            if let Some(end) = lowercase[start..].find("</title>") {
                return html.get(start..start + end).map(|s| s.trim().to_string());
            }
        }
        None
    }

    /// Parse JSON files
    async fn parse_json(&self, path: &Path) -> Result<(String, ParsedMetadata)> {
        let json_text = tokio::fs::read_to_string(path)
            .await
            .map_err(ToolError::Io)?;

        // Pretty print JSON
        let text = match serde_json::from_str::<Value>(&json_text) {
            Ok(value) => serde_json::to_string_pretty(&value).unwrap_or(json_text),
            Err(_) => json_text,
        };

        let metadata = ParsedMetadata {
            page_count: None,
            title: None,
            author: None,
        };

        Ok((text, metadata))
    }

    /// Parse XML files
    async fn parse_xml(&self, path: &Path) -> Result<(String, ParsedMetadata)> {
        let xml_text = tokio::fs::read_to_string(path)
            .await
            .map_err(ToolError::Io)?;

        // Extract text content from XML
        let mut text = String::new();
        let mut reader = quick_xml::Reader::from_str(&xml_text);
        let mut buf = Vec::new();

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(quick_xml::events::Event::Text(e)) => {
                    if let Ok(t) = e.unescape() {
                        let trimmed = t.trim();
                        if !trimmed.is_empty() {
                            text.push_str(trimmed);
                            text.push('\n');
                        }
                    }
                }
                Ok(quick_xml::events::Event::Eof) => break,
                Err(_) => break,
                _ => {}
            }
            buf.clear();
        }

        let metadata = ParsedMetadata {
            page_count: None,
            title: None,
            author: None,
        };

        Ok((text.trim().to_string(), metadata))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;
    use uuid::Uuid;

    #[tokio::test]
    async fn test_parse_text_file() {
        let mut temp_file = NamedTempFile::with_suffix(".txt").unwrap();
        writeln!(temp_file, "This is a test document.\nWith multiple lines.").unwrap();
        temp_file.flush().unwrap();

        let tool = DocParserTool;
        let session_id = Uuid::new_v4();
        let context = ToolExecutionContext::new(session_id);

        let input = serde_json::json!({
            "path": temp_file.path().to_str().unwrap()
        });

        let result = tool.execute(input, &context).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("This is a test document"));
        assert!(result.output.contains("multiple lines"));
    }

    #[tokio::test]
    async fn test_parse_markdown_file() {
        let mut temp_file = NamedTempFile::with_suffix(".md").unwrap();
        writeln!(temp_file, "# Header\n\nSome **bold** text.").unwrap();
        temp_file.flush().unwrap();

        let tool = DocParserTool;
        let session_id = Uuid::new_v4();
        let context = ToolExecutionContext::new(session_id);

        let input = serde_json::json!({
            "path": temp_file.path().to_str().unwrap()
        });

        let result = tool.execute(input, &context).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("# Header"));
        assert!(result.output.contains("**bold**"));
    }

    #[tokio::test]
    async fn test_parse_json_file() {
        let mut temp_file = NamedTempFile::with_suffix(".json").unwrap();
        writeln!(temp_file, r#"{{"name": "test", "value": 42}}"#).unwrap();
        temp_file.flush().unwrap();

        let tool = DocParserTool;
        let session_id = Uuid::new_v4();
        let context = ToolExecutionContext::new(session_id);

        let input = serde_json::json!({
            "path": temp_file.path().to_str().unwrap()
        });

        let result = tool.execute(input, &context).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("\"name\""));
        assert!(result.output.contains("\"test\""));
    }

    #[tokio::test]
    async fn test_parse_html_file() {
        let mut temp_file = NamedTempFile::with_suffix(".html").unwrap();
        writeln!(
            temp_file,
            "<html><head><title>Test Page</title></head><body><p>Hello World</p></body></html>"
        )
        .unwrap();
        temp_file.flush().unwrap();

        let tool = DocParserTool;
        let session_id = Uuid::new_v4();
        let context = ToolExecutionContext::new(session_id);

        let input = serde_json::json!({
            "path": temp_file.path().to_str().unwrap(),
            "include_metadata": true
        });

        let result = tool.execute(input, &context).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("Test Page"));
        assert!(result.output.contains("Hello World"));
    }

    #[tokio::test]
    async fn test_max_chars_truncation() {
        let mut temp_file = NamedTempFile::with_suffix(".txt").unwrap();
        writeln!(
            temp_file,
            "This is a very long document that should be truncated."
        )
        .unwrap();
        temp_file.flush().unwrap();

        let tool = DocParserTool;
        let session_id = Uuid::new_v4();
        let context = ToolExecutionContext::new(session_id);

        let input = serde_json::json!({
            "path": temp_file.path().to_str().unwrap(),
            "max_chars": 10
        });

        let result = tool.execute(input, &context).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("Truncated"));
    }

    #[tokio::test]
    async fn test_unsupported_format() {
        let mut temp_file = NamedTempFile::with_suffix(".xyz").unwrap();
        writeln!(temp_file, "Some content").unwrap();
        temp_file.flush().unwrap();

        let tool = DocParserTool;
        let session_id = Uuid::new_v4();
        let context = ToolExecutionContext::new(session_id);

        let input = serde_json::json!({
            "path": temp_file.path().to_str().unwrap()
        });

        let result = tool.execute(input, &context).await.unwrap();
        assert!(!result.success);
        assert!(result.error.unwrap().contains("Unsupported"));
    }

    #[tokio::test]
    async fn test_nonexistent_file() {
        let tool = DocParserTool;
        let session_id = Uuid::new_v4();
        let context = ToolExecutionContext::new(session_id);

        let input = serde_json::json!({
            "path": "/nonexistent/document.pdf"
        });

        let result = tool.execute(input, &context).await.unwrap();
        assert!(!result.success);
        assert!(result.error.unwrap().contains("not found"));
    }

    #[test]
    fn test_tool_schema() {
        let tool = DocParserTool;
        assert_eq!(tool.name(), "parse_document");
        assert!(!tool.requires_approval());

        let schema = tool.input_schema();
        assert!(schema.is_object());
        assert!(schema["properties"]["path"].is_object());
    }

    #[test]
    fn test_strip_html_tags() {
        let html = "<html><body><p>Hello</p><script>var x=1;</script><p>World</p></body></html>";
        let text = DocParserTool::strip_html_tags(html);
        assert!(text.contains("Hello"));
        assert!(text.contains("World"));
        assert!(!text.contains("var x"));
    }

    #[test]
    fn test_extract_html_title() {
        let html = "<html><head><title>My Document</title></head><body></body></html>";
        let title = DocParserTool::extract_html_title(html);
        assert_eq!(title, Some("My Document".to_string()));
    }
}
