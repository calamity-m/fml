# fml

hello

```
┌─────────────────────────────────────┬──────────────────────┐
│ Log Pane                            │ Info                 │
│                                     │──────────────────────│
│   [src-a] request timeout host=x    │ timestamp  ...       │
│   [src-b] pod restarted reason=oom  │ level      error     │
│ > [src-a] connection refused host=x │ message    conn...   │
│   [src-c] dial tcp: no such host    │ host       x         │
│   [src-a] retrying after backoff    │ source     src-a     │
│                                     │──────────────────────│
│                                     │ Preview              │
│                                     │──────────────────────│
│                                     │ [src-a] req started  │
│                                     │ [src-a] req timeout  │
│                                     │>[src-a] conn refused │
│                                     │ [src-a] retrying...  │
├─────────────────────────────────────┴──────────────────────┤
│ Query  conn refused                                        │
├────────────────────────────────────────────────────────────┤
│ SEARCH  src-a,src-b,src-c  3/120 matches                   │
└────────────────────────────────────────────────────────────┘
```

## Log Pane Functionality

### Terms:

- Head -> Oldest entry of the log store
- Tail -> Most recent entry of the log store
- Rendered Window -> Visible log lines in the log pane
- Rendered Head -> Top of the visible log lines
- Rendered Tail -> Bottom of the vissible log lines
- Retained Window -> Log lines the log pane has access to, which may extend past a Rendered Window, but not encompass a full ring buffer


## TODO

### 1. Refactor Search

We need to refactor the following portions of search:

-> History: The history search needs to emit more than once. Once we have a history  page, our sequence doesn't stop incrementing.
This can cause issues with the log_pane not displaying new results, e.g. if we enter into "history" mode before we have enough logs
in the buffer to fully saturate the rendered window.

-> Fuzzy: The fuzzy time bound shouldn't be a cut-off, but an emission point. If we are not complete, the fuzzy search should emit whatever was found at that point in time - essentially making it a tick rate again.

[x] History Search Tick Rate Added
[x] History Search Continuously Emits (If Changes have occured in the low/high bound)
[x] Fuzzy Tick rate added
[x] Fuzzy tick-rate related chunking state added to SearchState (Note: this is different from processing/scoring chunk rate)
[x] Fuzzy tick-rate related chunking chunk searching implemented

### 2. Setup Integration Testing

In order to avoid cyclic bug fixes causing tertiary issues, and causing a reliance on a human-in-the-loop tester, 
we should adopt snapshot testing at the app level, forming a series of integration tests. This will allow us to "replay"
a certain series of events, such as key inputs, ProducerEvents, SearchEvents, etc.

This integration test should rely on the app.rs setup, and use the app state and event bus to cause test cases we want to validate.
For example, we might want to test the ring buffer at capacity, and what different user actions would cause in the UI in that scenario. Custom helpers should be avoided where possible, and as much of the existing and real app code should be utilised. Test-specific helpers must be backed by a real need.

[x] Insta snapshot testing enabled
[x] Integration test for app.rs rendering created with Ratatui TestBackend, showing the whole tui
[x] Integration test added for ring buffer with maximum size + some amount (1 million/default), ensure app does not panic
[x] Integration test marked specifically, allowing them to be skipped for fast testing

### 3. Log Pane Tail Functionality

Note: Ignore scroll bar for now

The log pane should default to Tail mode, and display the latest results from the Tail search continuously.
This will need to touch the handle_search_event function, adding real logic to the handling of SearchResults,
rather than just dealing with setting latest request id. Future TODOs will build upon what we create here.
It's recommended to use the LogPaneState where possible. It is perfectly okay to refactor the LogPaneState if it does not currently meet the requirements needed to implement it's needed functionality.


[x] absolute_cursor migrated to TuiState
[x] Log pane displays results from the log store
[x] Log pane cursor at the bottom of the screen puts the TUI title into "Tail"
[x] Log pane continuously renders new incoming log lines, pushing older lines out of the Rendered Window
[x] Integration test added that validates live-log tailing works

### 4. Log Pane History Functionality

Note: Ignore scroll bar for now

FML is primarily a log-viewing application, and so scrolling through the log pane should work as you'd normally expect. 
When scrolling the cursor should travel upwards, but logs should remain static. When the cursor is at the top of the TUI's log pane, and the user continues to scroll, the log the user has highlighted will move down one, and the next line will be rendered at the top. This will require smart issuing of history buffers, ensuring we have enough of a Retained Window so that our request for a new retained window with a search query will not cause lag/delay.

e.g. if our retained window is [Seq_ID_LOW: 500, ..., Seq_ID_HIGH: 1000], once we reach half-way to Seq_ID, we may ask to fetch the window [Seq_ID_LOW:250, ..., Seq_ID_HIGH: 750].

[x] Log pane pauses when entering into history
[x] User can scroll to the bottom of the retained log store's low seq (scroll up) manually with repeated up-arrow keys
[x] User can scroll to the top of the log store's high seq (scroll down), re-entering into tail mode
[x] User can press Home to jump to the low bounds of the log store
[x] User can press End to jump to the high bounds of the log store and enter back into tail mode
[x] Integration test added for: 10k entries are populated into the log store, user inputs Home and should see sequence 1.
[x] Integration test added for: 100 entries are populated into the log store, user inputs 100 up-arrow inputs and sees sequence 1.
[x] Integration test added for: entering history pauses tail updates while new logs arrive.
[x] Integration test added for: pressing End from history returns to tail and renders the newest log.

### 5. Log Pane Scroll Bar Functionality

The scroll bar should be calculated based on the log store, not any retained window. We have a low, max and a cursor's sequence. We can then calculate the size of the scroll bar, and it's positioning.

[x] scroll bar reduces in size based on the amount of sequence ids retained, clamping to some minimum and maximum
[x] scroll bar scrolls based on the cursor in the log_pane
[x] integration test added to verify scrolling behaviour and size (top, mid, end) 

### 6. Log Pane Fuzzy Functionality

Note: Ignore scroll bar for now

Note: tail -> highest rank
      head -> lowest rank

FML's strength as a log viewing application will come from it's ability to naturally transition into fuzzy searching over 
log lines, allowing for users to triage issues in log output, search for proverbial needles in a haystack, as well as any general
high-level searching/analysis.

To do this we require the query box to be hooked into emission of a search query, scrolling through the ranked matches and displaying new matches as they arrive.

[ ] User can enter text into the query box to dispatch a `Query::Fuzzy`, using a debouncing algorithm
[ ] Emptying query/clearing search sets the log pane back to Tail mode
[ ] Log pane displays SEARCH for the mode title when fuzzy search dispatched
[ ] In search mode Fuzzy emissions are rendered as they arrive - as the fuzzy searching is done in chunks, and continuously lives for
live arrival of new highly matched entries, 
[ ] In search mode the fuzzy ranked matches are scrollable, the low/high bounds become the fuzzy ranked result
[ ] Home/End in search mode jumps to first/last fuzzy result.
[ ] Fuzzy match metdata is preserved into the TUI state, so that the next TODO can be implemented. Do not yet handle highlight rendering
[ ] Integration test added for submitting a fuzzy query and rendering ranked matches
[ ] Integration test added for fuzzy result navigation boundaries
[ ] Integration test added for exiting fuzzy mode by emptying the query box
[ ] (Integration or unit) Test added for debouncing algorithm, ensuring latest request id is observed

### 7. Log Pane Fuzzy Match Highlighting Functionality

In the previous todo the fuzzy search functionality was added, but users of FML would expect highlighting of their ranked matches
so that they can better refine their search, and have visual feedback to their matching. A key thing to note here is that the 
ratatui TestBackend's insta snapshots **does not support color**, we can get around this by having a configurable "highlight_match_style" which may 
be en enum of (Color, Underline, Bold, Block) or something - this will allow integration testing to confirm highlight functionality.

[ ] Configuration for highlight_match_style added to the TUI, ensuring it will be usable later by the InfoPane
[ ] Fuzzy search ranking is extended to cover source display name
[ ] Log line rendering replaces source-id with source display name
[ ] Highlighting of matched portions of entries in the LogPane added (message/level/display name)
[ ] Integration test added for highlighting, using a highlight_match_style that renders in the TestBackend

### 8. Log Pane Fuzzy Sticky Cursor Functionality

In order to keep the user focused and to not confuse them, we should modify the Log Pane fuzzy functionality to be sticky when possible. 
This will be a best effort. If the sequence remains in the re-ranked results, we should move the cursor to that sequence's new position. If the sequence vanishes, we should stay at the same rank index if possible. If not possible, clamp to the highest rank result.
There is one caveat here, where if our cursor is on the highest rank, whenever we get new results, our cursor should move to the highest rank, emulating "tailing" (not to be confused with tail mode/search)


[ ] Fuzzy selection is tracked by selected sequence id, not only by visible row/index
[ ] When a fuzzy emission arrives and the selected seq still exists, the cursor remains on that entry even if its rank changed
[ ] When the selected seq disappears, selection falls back to the previous rank index, clamped to the new result list
[ ] When there are no fuzzy results, SEARCH mode renders an empty result state with no selected seq
[ ] When the cursor is on the highest rank, and new results are added, the cursor stays at the new highest rank
[ ] Integration test added for live fuzzy re-rank preserving the selected entry
[ ] Integration test added for live fuzzy re-rank falling back when the selected entry disappears
[ ] Integration test added for fuzzy search, where our cursor is on the highest rank, and after a re-emission the
existing entry still exists, but drops from the highest rank. Our cursor moves to the next highest-rank, keeping it visually
pinned to the bottom of the rendered window.

### 9. Log Pane Fuzzy Scroll Bar Functionality

The scroll bar should operate in the fuzzy search too, with the bounds instead being the size of the fuzzy search result emission.

[ ] SEARCH mode scrollbar hides when fuzzy results fit into the rendered window
[ ] SEARCH mode scrollbar appears and uses fuzzy result count as content length when the results grow beyond the rendered window
[ ] SEARCH mode scrollbar position follows the sticky cursor
[ ] Integration test added for fuzzy scrollbar at first, middle and last result
[ ] Integration test added for fuzzy scrollbar with sticky cursor
