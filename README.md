# Mock LLM Server

Mock LLM Server is a stateless, zero-cost, fully-fleshed multi-modal LLM API simulation server designed specifically for the KeZen Engine.

Its core purpose is to **perfectly spoof LLM clients (like KeZen Engine or Cherry Studio) during automated testing (CI/CD) and local debugging by using pre-recorded "fixtures" (scripts). This allows developers to reproduce complex streaming (SSE) conversational scenarios without consuming real API credits.**

It strictly complies with the underlying SSE streaming protocols of **Anthropic (Claude)** and **OpenAI**, fully supporting pure text outputs, concurrent tool calls, mixed outputs, and rate-limit interruption simulation.

---

## Core Features

- 🎯 **Dual Protocol Support**: Built-in independent Anthropic and OpenAI streaming protocol generators to simulate native-level EventSource (SSE) interaction streams.
- 🔄 **Transparent Proxy Recording**: Acts as a man-in-the-middle proxy between the real network and KeZen. While seamlessly forwarding real traffic, it concatenates fragmented streaming data in the background and automatically exports persisted YAML test fixtures.
- 📦 **YAML Data-Driven**: No need to write any Rust testing mock code. Simply write or record YAML files to construct complex outlier behaviors or LLM function calls (Tool Calls).
- 🔌 **Zero-Invasive Testing**: Isolated from the main project as a Git Submodule. KeZen clients do not need any mock-related code refactoring; just change the Base URL to a localhost port.

---

## Quick Start

Mock LLM Server provides two main subcommands to perfectly cover the testing lifecycle.

### 1. Proxy (Recording Mode)
This is the most powerful feature for **painlessly generating complex business test fixtures**. Writing large Tool Call test cases manually is extremely tedious. 
With the proxy enabled, you can normally use KeZen to handle real, highly tricky tasks, and the server will automatically intercept the traffic and save it as YAML in the background.

```bash
cargo run --release --bin mock-llm-server -- proxy --port 8080 --out-dir fixtures_tmp
```

**Usage**:
1. Start Proxy mode (listens on port `8080` by default).
2. In your application configuration (e.g., `kezen.toml` or Cherry Studio), point the LLM API URL to `http://127.0.0.1:8080/v1` (keep the real API Key configuration).
3. Use the application normally to trigger various tasks containing Function Calls.
4. Check the `fixtures_tmp/` directory for auto-generated timestamped `.yaml` files. These files not only intercept and reconstruct the `MockResponse`, they also record your `request_payload` context, making it easy to review and reuse later.

### 2. Serve (Playback Testing Mode)
Once you have organized your recorded YAML scripts (usually moved to the formal `fixtures/` directory), you can disconnect from the internet and use them for pure automated test runs. The server will "blindly" play according to the script.

```bash
cargo run --release --bin mock-llm-server -- serve --port 3000 --script fixtures/demo_scenario.yaml --loop-mode
```

**Usage**:
- `--script` specifies a `.yaml` file or a directory full of `.yaml` scripts to load.
- Every time the client sends a request (regardless of the content), the server pops a response sequentially from the script queue and pushes it to the client.
- When `--loop-mode` is enabled, the server automatically loops back to the first script if the queue is exhausted. This is perfect for repeatedly debugging UI or a single logic path (highly recommended when connecting Cherry Studio). If not enabled, exhausting the script will return an HTTP 500 error.

---

## YAML Script Example

All YAML scripts are fundamentally defined by the `MockScriptEntry` sequence structure, supporting highly flexible return structure mutations.
Here is a mixed scenario where the "LLM first analyzes with some text, then decides to initiate two different tool calls concurrently":

```yaml
- kind: Response
  request_payload: 
    messages: [...] # (If recorded by Proxy, it automatically includes this so humans can traceback the context)
  blocks:
    # Segment 1: Pure text returning phase
    - type: Text
      text: |
        The log might be in /var/log. Let me find it using the bash tool.
    
    # Segment 2: System detects this is the Tool Calls streaming startup phase
    - type: ToolCall
      id: call_abc123
      name: SearchFile
      input:
        filename: "error.log"

    # Segment 3: Supports parallel Tool Calls (if the engine supports it)
    - type: ToolCall
      id: call_def456
      name: ExecuteBash
      input:
        command: "tail -n 100 /var/log/syslog"

- kind: Error
  status: 429
  message: "Too Many Requests"
```
*(Through this declarative building-block approach, you can easily forge behaviors like an HTTP 429/500 disconnect/reconnect, getting rejected by tools twice consecutively, or generating non-compliant JSON with hallucinations to precision-bomb test your Agent Engine's defense mechanisms.)*

---

## Gotchas / Pitfalls

When using the `mock-llm-server`, please keep the following caveats in mind:

1. **Strict Sequential Strictness (`serve` mode)**
   The `serve` mode is completely "deaf and blind". It does **not** look at your incoming `request_payload` (Prompt). It strictly pops the YAML entries sequentially in alphabetical file order.
   - **Gotcha**: If your KeZen Engine's test logic suddenly sends 3 backend requests instead of the expected 2, the test runner will desync and likely fail with an HTTP 500 on the third request (if `--loop-mode` is off). This strictness is intentional (Contract Testing).

2. **Dangling `fixtures_tmp` Records**
   The `proxy` mode records EVERYTHING. Every time you restart the Proxy and interact, it dumps new timestamped files.
   - **Gotcha**: Do not commit the `fixtures_tmp/` directory directly into your repository! It is ignored by Git. You must manually curate, rename, and move test-worthy behaviors into specific `fixtures/scenario_abc/` directories.

3. **Submodule Path Independence**
   Because `mock-llm-server` is a standalone Rust crate and a `git submodule` meant to be completely decoupled from KeZen's core business logic...
   - **Gotcha**: It does not import any KeZen domain types. The mock `ToolCall` inputs must be generic `serde_json::Value`. Do not attempt to rely on KeZen's specific struct definitions within the mock code.
