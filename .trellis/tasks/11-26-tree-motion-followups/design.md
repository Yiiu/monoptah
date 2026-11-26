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
