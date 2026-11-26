# Journal - Yiiu (Part 1)

> AI development session journal
> Started: 2026-11-24

---



## Session 1: Verify Native long conversation switching
<!-- trellis-session: v=2 fp=57d422d2bf9fe60e -->

**Date**: 2026-11-26
**Task**: Verify Native long conversation switching
**Branch**: `fix/native-long-session-performance`

### Summary

Verified faster initial history display and saved scroll restoration in release builds with isolated local samples. Recorded mixed scrolling measurements for the separate resource investigation.

### Git Commits

| Hash | Message |
|------|---------|
| `b6f49318` | perf(native): avoid rebuilding long history for session settings |
| `54ea0dc9` | perf(native): show long conversation history sooner |

### Status

[OK] **Completed**
