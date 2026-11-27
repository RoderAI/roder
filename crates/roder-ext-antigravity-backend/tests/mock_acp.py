#!/usr/bin/env python3
"""Deterministic official-ACP-shaped fixture; no credentials or network."""
import json
import os
import sys

root = os.path.dirname(os.path.realpath(__file__))
with open(os.path.join(root, "argv"), "a") as argv:
    argv.write(json.dumps(sys.argv[1:]) + "\n")
with open(os.path.join(root, "scenario")) as source:
    scenario = source.read()
session_id = "mock-" + str(os.getpid())
config = [
    {"id": "model", "category": "model", "type": "select", "currentValue": "gemini-one",
     "options": [{"group": "Google", "options": [
         {"value": "gemini-one", "name": "Gemini One"},
         {"value": "gemini-two", "name": "Gemini Two"}]}]},
    {"id": "thinking", "category": "thought_level", "type": "select",
     "currentValue": "low", "options": [{"value": "low", "name": "Low"},
                                        {"value": "high", "name": "High"}]},
]
pending = None


def emit(value):
    print(json.dumps({"jsonrpc": "2.0", **value}), flush=True)


def result(request_id, value):
    emit({"id": request_id, "result": value})


def update(value):
    emit({"method": "session/update", "params": {"sessionId": session_id, "update": value}})


def finish(request_id):
    update({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "hello "}})
    update({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "subscription"}})
    result(request_id, {"stopReason": "end_turn", "usage": {
        "inputTokens": 100, "cachedReadTokens": 20, "outputTokens": 30}})


for line in sys.stdin:
    message = json.loads(line)
    with open(os.path.join(root, "trace"), "a") as trace:
        trace.write(json.dumps(message) + "\n")
    method = message.get("method")
    params = message.get("params", {})
    request_id = message.get("id")
    if method == "initialize":
        if scenario == "hang":
            continue
        result(request_id, {"protocolVersion": 1,
                           "authMethods": [{"id": "oauth-personal"}],
                           "agentCapabilities": {"loadSession": scenario != "no-load",
                                                 "auth": {"logout": scenario != "no-logout"},
                                                 "sessionCapabilities": {"list": {}}}})
    elif method == "authenticate":
        if scenario == "auth-failure":
            emit({"id": request_id, "error": {"code": -32603,
                  "message": "secret-must-not-appear", "data": "secret-must-not-appear"}})
        else:
            result(request_id, {})
    elif method in ("session/new", "session/load"):
        if scenario == "session-failure":
            emit({"id": request_id, "error": {"code": -32000, "message": "secret-must-not-appear"}})
            continue
        if method == "session/load":
            session_id = params["sessionId"]
            update({"sessionUpdate": "user_message_chunk",
                    "content": {"type": "text", "text": "previous question"}})
            update({"sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": "previous answer"}})
        modes = [{"id": "default", "name": "Default"}]
        if scenario == "supports-plan":
            modes.append({"id": "plan", "name": "Plan"})
        result(request_id, {"sessionId": session_id, "configOptions": config,
                           "modes": {"currentModeId": "default", "availableModes": modes}})
    elif method == "session/set_mode":
        result(request_id, {})
    elif method == "session/set_config_option":
        for option in config:
            if option["id"] == params["configId"]:
                option["currentValue"] = params["value"]
        result(request_id, {"configOptions": config})
    elif method == "session/list":
        result(request_id, {"sessions": [{"sessionId": "existing", "title": "Previous", "cwd": root}]})
    elif method == "logout":
        result(request_id, {})
    elif method == "session/prompt":
        text = params["prompt"][0]["text"]
        pending = request_id
        if text == "cancel":
            continue
        if text in ("approval", "question"):
            emit({"id": 99, "method": "session/request_permission", "params": {
                "sessionId": session_id,
                "toolCall": {"toolCallId": "interaction_question" if text == "question" else "tool",
                             "title": "Choose" if text == "question" else "Run command",
                             "kind": "execute"},
                "options": [{"optionId": "yes-exact", "name": "Yes", "kind": "allow_once"},
                            {"optionId": "no-exact", "name": "No", "kind": "reject_once"}]}})
        elif text == "unsupported":
            emit({"id": 100, "method": "fs/read_text_file",
                  "params": {"sessionId": session_id, "path": "/secrets"}})
        else:
            finish(request_id)
            pending = None
    elif method == "session/cancel":
        if pending is not None:
            result(pending, {"stopReason": "cancelled"})
            pending = None
    elif method is None and request_id in (99, 100):
        if pending is not None:
            finish(pending)
            pending = None
