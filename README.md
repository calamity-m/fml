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

[] History Search Tick Rate Added
[] History Search Continuously Emits (If Changes have occured in the low/high bound)
[] Fuzzy Tick rate added
[] Fuzzy tick-rate related chunking state added to SearchState (Note: this is different from processing/scoring chunk rate)
[] Fuzzy tick-rate related chunking chunk searching implemented

### 2. Setup Integration Testing

In order to avoid cyclic bug fixes causing tertiary issues, and causing a reliance on a human-in-the-loop tester, 
we should adopt snapshot testing at the app level, forming a series of integration tests. This will allow us to "replay"
a certain series of events, such as key inputs, ProducerEvents, SearchEvents, etc.

This integration test should rely on the app.rs setup, and use the app state and event bus to cause test cases we want to validate.
For example, we might want to test the ring buffer at capacity, and what different user actions would cause in the UI in that scenario. Custom helpers should be avoided where possible, and as much of the existing and real app code should be utilised. Test-specific helpers must be backed by a real need.

[] Insta snapshot testing enabled
[] Integration test for app.rs rendering created with Ratatui TestBackend, showing the whole tui
[] Integration test added for ring buffer with maximum size + some amount (1 million/default), ensure app does not panic
[] Integration test marked specifically, allowing them to be skipped for fast testing

### 3. Log Pane Tail Functionality

Note: Ignore scroll bar for now

The log pane should default to Tail mode, and display the latest results from the Tail search continuously.

[] Log pane displays results from the log store
[] Log pane cursor at the bottom of the screen puts the TUI title into "Tail"
[] Log pane continuously renders new incoming log lines, pushing older lines out of the Rendered Window
[] Integration test added that validates live-log tailing works

### 4. Log Pane History Functionality

Note: Ignore scroll bar for now

FML is primarily a log-viewing application, and so scrolling through the log pane should work as you'd normally expect. 
When scrolling the cursor should travel upwards, but logs should remain static. When the cursor is at the top of the TUI's log pane, and the user continues to scroll, the log the user has highlighted will move down one, and the next line will be rendered at the top. This will require smart issuing of history buffers, ensuring we have enough of a Retained Window so that our request for a new retained window with a search query will not cause lag/delay.

e.g. if our retained window is [Seq_ID_LOW: 500, ..., Seq_ID_HIGH: 1000], once we reach half-way to Seq_ID, we may ask to fetch the window [Seq_ID_LOW:250, ..., Seq_ID_HIGH: 750].

[] Log pane pauses when entering into history
[] User can scroll to the bottom of the retained log store's low seq (scroll up) manually with repeated up-arrow keys
[] User can scroll to the top of the log store's high seq (scroll down), re-entering into tail mode
[] User can press Home to jump to the low bounds of the log store
[] User can press End to jump to the high bounds of the log store and enter back into tail mode
[] Integration test added for: 10k entries are populated into the log store, user inputs Home and should see sequence 1.
[] Integration test added for: 100 entries are populated into the log store, user inputs 100 up-arrow inputs and sees sequence 1.
[] Integration test added for: ... (others as needed/required)

### 5. Log Pane Scroll Bar Functionality

The scroll bar should be calculated based on the log store, not any retained window. We have a low, max and a cursor's sequence. We can then calculate the size of the scroll bar, and it's positioning.

[] scroll bar reduces in size based on the amount of sequence ids retained, clamping to some minimum and maximum
[] scroll bar scrolls based on the cursor in the log_pane
[] integration test added to verify scrolling behaviour and size (top, mid, end) 

### 6. Log Pane Fuzzy Functionality

Note: Ignore scroll bar for now

TBD!
