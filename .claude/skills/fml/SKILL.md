---
name: fml
description: Fetch log lines from a running or recorded feed, such as docker/kubernetes/file/stdout. Ideal for tracing problems through complex deployments or discovering issues.
---

# /fml — Query logs with fml

Fetch log lines from a running or recorded feed using `fml --headless` and analyse them.

## Usage

```
/fml [query] [--feed <feed>] [--greed <0-10>] [--tail <n>] [--namespace <ns>] [--container <name>]
```

## Steps

1. Parse the arguments from `$ARGUMENTS`:
   - Extract `--feed` (default: `kubernetes`), `--greed` (default: `4`), `--tail` (default: `100`), `--namespace`, `--container`.
   - Treat any remaining bare words as the query expression.
   - If no query is provided, ask the user what they want to search for.

2. Run fml in headless mode:
   ```bash
   fml --headless \
     --feed <feed> \
     --query "<query>" \
     --greed <greed> \
     --tail <tail> \
     --format jsonl \
     [--namespace <namespace>] \
     [--container <container>]
   ```

3. Pass the output to Claude with the prompt:
   > Here are the log lines matching `<query>` (greed=<greed>, last <tail> lines). Analyse them: identify patterns, likely root causes, and any actionable next steps.

4. Present Claude's analysis to the user. If the user wants to drill deeper, suggest a follow-up `/fml` invocation with a more targeted query or higher greed.

## Examples

```
/fml level:error
/fml timeout --greed 7 --tail 200
/fml --feed docker --container api exception --tail 500
/fml --feed kubernetes --namespace payments --greed 8 deadlock
```
