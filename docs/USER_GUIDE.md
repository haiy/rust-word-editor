# User guide

[English](USER_GUIDE.md) · [简体中文](USER_GUIDE.zh-CN.md)

Use **New**, **Open**, **Save** and **Save As** in the top bar. DOCX is the main document format. For imported documents containing unsupported features, save a copy so that the original stays intact. Draft recovery is offered at startup after an interrupted editing session.

Select text before changing its font, applying a style or using selection-based AI rewriting. AI Chat works without a selection. Templates create a new editable document. Personal templates and reusable assets are stored locally.

Choose **English / 中文** to change the interface. The app retains the current document, presentation and media, but resets undo history and unapplied AI suggestions after confirmation. Document text and file names remain unchanged.

In **AI Service**, choose locally authenticated Codex or enter your own compatible API endpoint, model and key. In **Chat**, enter a request and continue asking follow-up questions. Attach document text with **Reference selection** or **Reference document**. Answers stay in the conversation until you copy, insert or replace a referenced selection. Each conversation supports 20 turns and stays available when the sidebar closes or the interface language changes; restarting clears chat history. In **Rewrite selection**, generate a suggestion, review it and apply it. For whole-document translation/formatting, open **Tools**. A changed source document invalidates an old AI preview.

Open **PPT Templates** to choose an original starter, or **Open PPTX** to import your own presentation. Edit titles, paragraphs and slides; export a PPTX to save the presentation. Editing imported presentations preserves existing artwork but does not expose all shape properties.

**PDF** and **Image** tools have separate previews. Export their results or insert supported output into the Word document. PDF-to-Word produces editable text, not an exact replica of every page layout.

For **Cloud Storage**, configure rclone first, then select a remote and a folder. Alternatively connect a WebDAV endpoint or a local sync folder. Uploading creates a separate file, checks conflicts and verifies the readback. A read-only connection disables uploads. Provider limits, quotas and permissions still apply.
