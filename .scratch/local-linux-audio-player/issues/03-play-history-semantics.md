# Define play-history and configurable played threshold behavior

> Migrated to [GitHub issue #4](https://github.com/itzam-na/kulkan/issues/4). This is a local snapshot; update the GitHub issue.

Type: grilling
Status: resolved

Precondition resolved by [Define library scanning and media-file identity behavior](https://github.com/itzam-na/kulkan/issues/5).

## Question

The default played threshold is 75% of a track and is configurable. What should count toward that threshold during seeking, pausing, skipping, replaying, and changing the configured value? How should local play history be represented and persisted so the not-yet-played criterion behaves predictably? Resolve file identity and history association for moved or replaced files in the blocking ticket first.

## Answer

Use one app-wide integer threshold from 1–100%, default 75%. A Playback attempt qualifies when the listener hears the configured share of distinct Track content; pauses preserve coverage, seeks over content do not add it, and replaying heard portions does not count twice. The count increments immediately on reaching the threshold, at most once per attempt; skipping before it adds nothing, while skipping afterward keeps the count. Each repeat pass is a new attempt. Changing the setting affects future attempts only and does not recalculate history or alter an in-progress attempt. If duration is unknown, only natural completion qualifies. Persist aggregate counts locally per Library entry, without timestamps; an unknown count is zero. Users may clear one entry or all history, including inactive entries. See [GitHub issue: Define play-history and configurable played threshold behavior](https://github.com/itzam-na/kulkan/issues/4).
