# Isolate Playback attempt qualification

**Status: accepted**

Place Playback attempt state, playback-snapshot interpretation, heard-content tracking, and qualification rules in a private `kulkan-runtime` module. Keep attempt lifecycle and play-count persistence with Queue control, which records through Library state and retries after persistence failures; this keeps qualification independently testable without coupling it to Library persistence.

Preserve the existing rules: count distinct heard content, capture the threshold at attempt start, qualify immediately and at most once, require natural completion when duration remains unknown (a seek to the end is insufficient), and count an attempt that reaches the threshold before playback fails. Add focused tests for the qualification module and retain the queue/playback integration tests for lifecycle and persisted counts.

## Considered options

- **Keep qualification in Queue control:** rejected because attempt rules and queue orchestration would continue to change and be tested together.
- **Move play-count persistence into the qualification module:** rejected because Library persistence and retry behavior belong with Queue control's coordination of playback and Library state.
