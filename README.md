# convey

English | [日本語](README.ja.md)

Convey turns repeatable prompt composition into an interactive terminal
workflow. Describe the information you want to collect in YAML, fill it in from
a keyboard-driven interface, and send the rendered Markdown directly to a
selected terminal pane.

It is useful when a prompt follows a known structure but depends on live local
context—for example, choosing a Kubernetes context, namespace, and resource
before asking an agent to investigate it.

![demo](assets/convey-demo.gif)

## Motivation

I often ask Claude or Codex to investigate issues using Kubernetes and Grafana.
For example, I might notice an anomaly in Grafana and ask one of them to find
out why a Pod is in `CrashLoopBackOff`.

Simply saying "investigate this Pod" is not enough to get started. I need to
specify which `context` to use, which `namespace` it is in, its `kind`, and the
Pod name. These are not details I want the agent to guess; they are choices I
want to make as the person requesting the investigation.

Whenever the target changed, however, I would look up the `context`,
`namespace`, resource kind, and Pod name with `kubectl`, copy each one, and fill
them into another prompt with almost the same structure. Each operation is
small, but repeating them for every investigation is tedious. There is also a
risk of pasting the wrong `context` or `namespace`, and this input work before
the agent can even begin had become a bottleneck.

I often wished I could list the values available in my local environment,
select the ones I needed in sequence, insert them into my usual request, and
send it directly to an agent that was already running. I wanted to decide the
investigation target myself while eliminating only the work of transcribing
that decision into a prompt.

I built Convey to remove this friction. It defines the values that change each
time as inputs and the stable request structure as a YAML workflow, making the
initial input before an agent starts investigating much smoother.

## How this differs from `SKILL.md`

A `SKILL.md` can instruct an agent to ask the user for required values. However,
it is still an instruction to the model and does not guarantee that every
required choice will be made.

Convey does not send the request until the user has selected the values they do
not want the agent to decide. It acts as a gate that ensures required human
decisions are made before the request reaches the agent.

## Features

- YAML-defined forms with `select` and multiline `textarea` inputs
- Static select choices or dynamically loaded choices from local commands
- Dependent inputs that reload when an upstream value changes
- Handlebars templates for generating Markdown
- Required and optional input validation
- A searchable tree of terminal applications, windows, tabs, and panes
- Workflow selection from a directory, or direct launch of one workflow file
- Seamless keyboard navigation across destinations, workflows, fields, and
  select candidates
- Direct delivery and submission to Ghostty, iTerm2, and tmux panes
- In-place errors and retries for failed candidate commands and destination
  discovery

## Requirements

- One of the supported destinations:
  - tmux on a platform where the `tmux` command is available
  - Ghostty or iTerm2 on macOS
- For Ghostty and iTerm2, permission for your terminal to control the
  destination application through macOS Automation when prompted
- Any programs used by workflow candidate commands, such as `kubectl`

Ghostty, iTerm2, and tmux are the currently supported destinations. The tmux
integration invokes the `tmux` command directly and therefore works without
platform-specific application automation.

## Installation

Install Convey from its Homebrew tap:

```console
$ brew install ynqa/tap/convey
```

To build from source instead, install Rust 1.85 or newer and run:

```console
$ cargo install --git https://github.com/ynqa/convey.git --locked
```

## Quick start

```console
$ git clone https://github.com/ynqa/convey.git
$ convey --terminal ghostty convey/examples
```

Replace `ghostty` with `iterm2` or `tmux` for those destinations. The input
screen will let you choose a destination pane, choose a YAML file from
`convey/examples`, complete its fields, and submit the rendered Markdown.

You can also launch one workflow directly. The workflow selector remains in
the interface, with the supplied file already selected:

```console
$ convey --terminal ghostty convey/examples/kubernetes-investigation.yaml
```

The positional `WORKFLOW` argument accepts either a workflow file or a
directory. Directory mode lists the `.yaml` and `.yml` files directly inside
that directory.

### Selecting terminal applications

Use `-t` or `--terminal` to control which applications Convey queries:

```console
$ convey --terminal ghostty examples
$ convey --terminal iterm2 examples
$ convey --terminal tmux examples
$ convey --terminal ghostty,iterm2 examples
```

The option defaults to all supported applications. Specifying the application
you actually use avoids querying an unavailable application.

## How the input screen works

The screen is organized into three connected sections:

1. **Destination** — the terminal pane that will receive the result
2. **Workflow** — the YAML definition to run
3. **Input fields** — the values used to render the workflow

The markers show the state of each section or field:

- `▶` focused
- `✓` saved
- `○` pending

Navigation continues through candidates and section boundaries instead of
treating each selector as an isolated control. Crossing a boundary saves the
current value.

| Key | Action |
| --- | --- |
| `Tab` or `Down` | Move to the next candidate, textarea line, field, or section |
| `Shift+Tab` or `Up` | Move to the previous candidate, textarea line, field, or section |
| `Enter` | Save the current destination, workflow, or select value and move forward |
| `Enter` in a textarea | Insert a newline |
| `Ctrl+Enter` in a textarea | Save the textarea and move forward |
| `Ctrl+S` | Save the focused value, validate required inputs, submit, and reset the form |
| `Ctrl+R` | Refresh destinations or retry a failed candidate command |
| `Ctrl+G` | Open or close the help page |
| `Esc` | Return from help, or cancel from the input screen |
| `Ctrl+C` | Cancel |

The mouse can also focus sections, position an editor cursor, and choose a
visible candidate.

Required inputs do not prevent navigation. Convey validates them when you
submit with `Ctrl+S` and moves focus to the first missing value.

If a select query has no matching candidate, pressing `Enter` saves the
non-empty query as a custom value.

### Searching destinations

Typing while Destination is focused filters the terminal tree. Unqualified
terms search across the application, pane name, working directory, and pane
location. Multiple terms use AND semantics.

Qualify a term to search a specific property:

| Qualifier | Property |
| --- | --- |
| `app:` | Terminal application |
| `name:` | Pane name |
| `cwd:` or `path:` | Working directory |
| `w:` or `window:` | Window index |
| `t:` or `tab:` | Tab index |
| `p:` or `pane:` | Pane index |
| `id:` | Application-specific pane identifier |

Quote values containing spaces:

```text
app:ghostty cwd:/workspace name:"Claude Code"
```

Non-matching panes remain visible but dimmed so that matches retain their
application, window, and tab context.

## Writing a workflow

A workflow contains a display name, an ordered map of inputs, and an output
template:

```yaml
name: incident-summary

inputs:
  environment:
    type: select
    candidates:
      values: [development, staging, production]

  request:
    type: textarea
    allow_empty: false

output:
  template: |
    # Incident request

    - Environment: `{{ inputs.environment }}`

    {{ inputs.request }}
```

Input definitions are displayed in YAML order. A workflow must define at least
one input and a non-empty output template.

### Input types

#### `textarea`

Use a textarea for free-form, multiline input:

```yaml
request:
  type: textarea
  allow_empty: false
```

Inputs allow empty values by default. Set `allow_empty: false` to require a
value when submitting.

#### `select` with static candidates

Declare a fixed list with `candidates.values`:

```yaml
priority:
  type: select
  candidates:
    values: [low, medium, high]
```

#### `select` with command candidates

Load candidates from a local program with `candidates.command`:

```yaml
context:
  type: select
  candidates:
    command:
      program: kubectl
      args: [config, get-contexts, -o, name]
```

Convey runs the program directly without a shell. Each non-empty line of UTF-8
stdout becomes one candidate. A non-zero exit status remains visible in the
form and can be retried with `Ctrl+R`.

### Dependent candidates

A command-backed select can depend on values saved by earlier inputs. Template
expressions are supported in both the command program and its arguments:

```yaml
namespace:
  type: select
  depends_on: [context]
  candidates:
    command:
      program: kubectl
      args:
        - --context
        - "{{ inputs.context }}"
        - get
        - namespaces
        - -o
        - 'jsonpath={range .items[*]}{.metadata.name}{"\n"}{end}'
```

The command waits until all dependencies are saved. Changing an upstream value
clears and reloads every affected downstream select. Dependencies must refer to
earlier inputs; textareas cannot declare dependencies.

### Output templates

`output.template` is rendered with Handlebars. Access collected values through
`inputs.<name>`:

```yaml
output:
  template: |
    Investigate `{{ inputs.resource }}` in `{{ inputs.namespace }}`.

    {{ inputs.request }}
```

Unset optional inputs render as empty strings. The generated text is emitted as
Markdown without HTML escaping.

See
[`examples/kubernetes-investigation.yaml`](examples/kubernetes-investigation.yaml)
for a complete workflow with several dependent command-backed inputs.

## Delivery and safety

Submitting with `Ctrl+S` sends the rendered text to the selected pane and then
sends Enter as a separate automation action. After a successful submission,
Convey returns to a fresh input screen so that another message can be composed.
In a shell, the destination may therefore execute the rendered text as a
command. Confirm the selected pane and review workflows before submitting them.

Candidate programs run locally with the same user permissions as Convey.
Although Convey invokes them without a shell, workflow authors still control
which executable and arguments are run. Use workflows only from sources you
trust.

## Development

Run the formatter, tests, and lints before submitting a change:

```console
$ cargo fmt --check
$ cargo test
$ cargo clippy --all-targets -- -D warnings
```

The Ghostty and iTerm2 integrations are implemented as standalone AppleScripts
under `src/automation/ghostty` and `src/automation/iterm2`. They can be run
directly with `osascript` when debugging destination discovery or delivery.
The platform-independent tmux integration under `src/automation/tmux` invokes
the `tmux` CLI directly.

### Releasing

The generated release workflow builds both macOS architectures and publishes
the Homebrew Formula to `ynqa/homebrew-tap` when a version tag such as
`v0.1.0` is pushed. The repository must define a `HOMEBREW_TAP_TOKEN` Actions
secret with write access to that tap.
