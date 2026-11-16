# Make playback from a List explicit and Queue-aware

**Status: accepted**

Double-clicking a Queue entry plays that Queue occurrence without changing Queue order. Double-clicking an active track in a Library List applies the persisted List playback behavior:

- **Disabled** ignores list activation.
- **Substitute Queue** replaces the Queue with the clicked track and following active tracks in the current List, then starts the clicked track.
- **Replace first** replaces the current Queue entry, or the first Queue entry when playback is stopped, and starts the clicked track while preserving the remaining entries.
- **Listen Next** inserts the clicked track immediately after the current entry and lets the current track finish. With no current entry, it starts the inserted track.
- **Add to Queue** appends the clicked track and starts it only when the Queue was empty.
- **Ask me** uses List substitution when the Queue is empty. When the Queue is nonempty, it presents the behavior choices before changing playback. Choosing **Substitute Queue** confirms that the current Queue will be erased. The listener may remember the selected behavior.

Default to **Ask me** so a double-click cannot silently erase an existing Queue. Persist the behavior with app state so it survives relaunch, import/export, and reset.

## Considered options

- **Treat every List double-click as Queue substitution:** rejected because it silently discards a listener-managed Queue.
- **Always append a single clicked track:** rejected because it does not provide the requested implicit continuation through the current List.
- **Keep List playback behavior only in the UI process:** rejected because the setting must persist with the rest of app state and participate in backup/reset.
