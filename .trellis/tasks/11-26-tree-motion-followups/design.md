# Tree motion

- Leaving Tree clears the held unread looks and fades. Within Tree, the
  focused session keeps its unread look while that conversation is the
  visible destination. App pages, Settings, Search and Mono conversations
  settle the old row through the existing 200 ms fade.
- Wheel scrolling stays available while dragging. Each block uses measured
  row heights where available and the virtual list's size estimates elsewhere.
  The scroll delta since the press places both the pointer and the block
  measurements in the same coordinates. The mouse-up event samples the
  latest position and scroll state before committing the order.
- The existing sidebar frame loop scrolls the list near either edge. Speed
  scales with how far the pointer is inside the edge zone. The lift plate
  is drawn on the first normal-motion drop frame and stays through any
  settlement. Reduce motion applies each movement and drop immediately.

## Windows Native Dev verification

The Windows 11 Native Dev run used an isolated sample workspace and Rust
1.98.1. A temporary in-app probe called the normal layout and Inbox entry
points and dispatched GPUI pointer and wheel events on the native window.
Windows Computer Use could not connect to its local pipe, so physical
pointer control remained unverified. The probe source stayed outside the
committed build.

| Requirement | Observed result |
| --- | --- |
| R1 | Switched from Tree to Classic, opened another conversation, and returned to Tree. The first conversation immediately used its read appearance. |
| R2 | Opened Inbox while a finished conversation remained the active session. Its held unread appearance eased away over the existing fade. |
| R3 | A 55px wheel scroll increased the drag offset by 55px and kept the lifted block under the pointer. Holding at the bottom moved the list from 55px to 289.15px. Holding at the top brought it back to 124.91px. The lift plate remained on release and cleared after the drop settled. |

The final GPUI test binary also passed all seven motion regressions and
the original Classic project and group reorder test. The wheel regression
asserted a 180px scroll, fixed header position, and final project order.
