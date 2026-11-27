export function deferred() {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

// Consumers may stop reading without settling the backend's terminal result.
export class EventQueue {
  #items = [];
  #bytes = 0;
  #waiter;
  #closed = false;

  push(event) {
    if (this.#closed) return;
    const bytes = Buffer.byteLength(JSON.stringify(event));
    if (this.#items.length >= 4096 || this.#bytes + bytes > 16 * 1024 * 1024) {
      throw new Error("Roder output consumer is not keeping up");
    }
    this.#items.push({ event, bytes });
    this.#bytes += bytes;
    this.#waiter?.resolve();
    this.#waiter = undefined;
  }

  end(discard = false) {
    this.#closed = true;
    if (discard) { this.#items = []; this.#bytes = 0; }
    this.#waiter?.resolve();
    this.#waiter = undefined;
  }

  async *[Symbol.asyncIterator]() {
    while (true) {
      const item = this.#items.shift();
      if (item) {
        this.#bytes -= item.bytes;
        yield item.event;
      } else if (this.#closed) {
        return;
      } else {
        this.#waiter = deferred();
        await this.#waiter.promise;
      }
    }
  }
}

export function runtimeEvent(update) {
  const tag = update?.sessionUpdate;
  if (tag === "agent_message_chunk" || tag === "agent_thought_chunk") {
    if (update.content?.type !== "text") return undefined;
    return {
      type: "text_delta", text: update.content.text,
      stream: tag === "agent_message_chunk" ? "output" : "thought", tag,
    };
  }
  if (tag === "tool_call" || tag === "tool_call_update") {
    const content = (update.content ?? []).flatMap((item) => {
      if (item.type === "content" && item.content?.type === "text") return [item.content.text];
      if (item.type === "diff") return [`${item.path}\n${item.newText}`];
      return [];
    });
    return {
      type: "tool_call", tag,
      text: [update.title ?? update.status ?? "Tool update", ...content].join("\n"),
      toolCallId: update.toolCallId, title: update.title,
      kind: update.kind, status: update.status,
    };
  }
  if (tag === "usage_update") {
    return { type: "status", tag, text: "Context usage", used: update.used, size: update.size };
  }
  if (tag === "plan") {
    return {
      type: "status", tag,
      text: (update.entries ?? []).map((entry) => `[${entry.status}] ${entry.content}`).join("\n"),
    };
  }
  if (tag === "current_mode_update") {
    return { type: "status", tag, text: `Mode: ${update.currentModeId}` };
  }
  if (tag === "config_option_update") {
    return { type: "status", tag, text: "Session configuration updated" };
  }
  if (tag === "available_commands_update") {
    return {
      type: "status", tag,
      text: (update.availableCommands ?? []).map((command) => `/${command.name}: ${command.description}`).join("\n"),
    };
  }
  if (tag === "session_info_update") {
    return { type: "status", tag, text: update.title ?? "Session information updated" };
  }
  return undefined;
}

export function promptBlocks(text, attachments = []) {
  if (typeof text !== "string" || !Array.isArray(attachments)) {
    throw new Error("A turn needs text and an optional attachments array");
  }
  return [
    { type: "text", text },
    ...attachments.map(({ mediaType, data }) => {
      if (!["image/png", "image/jpeg", "image/webp", "image/gif"].includes(mediaType)
          || typeof data !== "string" || data.length === 0
          || data.length % 4 !== 0 || !/^[A-Za-z0-9+/]+={0,2}$/.test(data)
          || Buffer.from(data, "base64").toString("base64") !== data) {
        throw new Error("Attachments must be base64 PNG, JPEG, WebP or GIF images");
      }
      return { type: "image", mimeType: mediaType, data };
    }),
  ];
}

export function runtimeError(code, message, detailCode) {
  return Object.assign(new Error(message), { name: "AcpRuntimeError", code, detailCode });
}
