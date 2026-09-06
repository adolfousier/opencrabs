-- Markdown-plane host bodies for pending follow-ups (#79, A1 piece 4).
-- Table-bearing answers can now carry the suggestion controls: the merge
-- edit rides the rich MARKDOWN plane (editMessageText + rich_message.markdown),
-- whose server-side render keeps tables intact — the HTML plane flattens
-- them (#679), which is why those answers were excluded from merging.
-- The merged markdown payload is stored alongside the html copy so every
-- later edit of the bubble (pick redraw, stale strip) re-sends it through
-- the same markdown plane instead of flattening the table.
-- Nullable: NULL = html-plane host (all rows written before this change).

ALTER TABLE pending_followups ADD COLUMN host_markdown TEXT;
