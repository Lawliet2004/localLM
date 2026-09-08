# Conversation exports

Open a saved conversation and use **Export conversation** in its header. Choose Markdown (`.md`) for reading or JSON (`.json`) for structured data in the native Save dialog. Export is unavailable while an operation is active. Cancelling the dialog writes nothing; a successful write shows its destination in the application.

JSON schema version 1 includes the conversation identity and title, export timestamp, selected connector/local tools, and every message with its original ID, role, content, reasoning, status, saved error and timestamp. Timestamps are Unix milliseconds. Tool messages retain their serialized request/result audit. This is an export format; importing a conversation is not currently implemented.

Markdown includes the title, conversation identity, timestamps, response statuses, reasoning and message content. Tool audits use literal code blocks with fences long enough to contain embedded Markdown fences. Normal message Markdown is preserved.

Exports contain conversation data, which may include private content supplied by you or returned by tools. Application credentials and unrelated runtime settings are not added to the export. Store and share the resulting file as you would the conversation itself.

Writes use a temporary file in the destination folder, flush it, then replace the selected destination. Unsupported extensions, missing directories and write errors are reported. Rendered output is limited to 64 MiB. Crash durability across a power failure depends on the operating system and filesystem; this is not a backup or synchronization service.
