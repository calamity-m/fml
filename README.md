# fml

This is really a test project for myself to re-envision how i handle log triage and issue resolution. Right now I don't have a LGTM stack that's dependable as I imagine a bunch of others don't either. Additionally, I seem to continually get people patting me on the shoulder with minimal info, in which I immediately find the issue by correlating 2-3 different logs.

fml should help, somehow, in this regard. If it doesn't, idk, use some other tool. LGTM stak or splunk is the best for this but also, I like staying in my terminal and am tired of constantly having 10 tabs open for 10 different queries, etc.

lnav is also good but my workflow doesn't really fit it. I basically play the human searching through the haystack to find the needle. It probably is inefficient but i don't think everyone can stand atop their SRE team and say "giveth to me thy errors, illuminate the way upon the world which you control". I dunno if this makes sense, but I do a DFS mixed with temporary BFS, idk fuck its been so long since i did any algorithms:

1. Find an interesting thread like an error log, or something related to a search, muh bfs.
2. go deep into thread to some n depth that feels good
3. discover related events to that depth, either walk down more or walk up and go back down

no tool seems to help me do this naturally currently other than opening 5 browser tabs or 5 terminals and having a complete mess of a workspace.

## Theory

I feel like having a nvim/helix editor style is the most intuitive to me, maybe that's because I'm ultimately opening tons of log streams into editors, but it lets me split and highlight particular sections. I don't need to copy an entire log, only the relevant portion i care about.

fml can extend on this by allowing for panes to be split with, or tabs to be created with, 'filter' logic. i.e. some correlation, like same 'request-id', same source, time gated, etc.

or just the same log stream, to allow me to search for another needle.

## AI Disclaimer

This originally started as a much more human-organic project, but i'm slow and losing steam. A lot of AI is now being used to develop this project, though I'm generally keeping control and still employ oversight. You've been warned though, if you hate AI-developed projects it's best to leave now.
