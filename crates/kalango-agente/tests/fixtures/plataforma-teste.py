#!/usr/bin/env python3
"""A test double of the SERVICE only (the agents platform, kalango-fluxosd), never of the model.

kalangod runs `<this> chamar METHOD JSON --socket S` for platform.preview, platform.apply and agents.list. This keeps
what was saved in $PLATAFORMA_TESTE_DIR (agents.json, flows.json, chamadas.jsonl) and refuses what the real service's
serde types and `Agent::issues` / `Flow::issues` refuse (ported from kalango-fluxos, fluxos-core: reachability from the
entry, non-empty agent ids, approval timeouts, decide routes, sandbox rules, sub-agent shapes), with the same words: a missing or non-text id, an unknown block type (the list comes from
$PLATAFORMA_TESTE_TIPOS, the engine's `Block` tags), a block without the data it needs, an edge to a block that does
not exist. It does not check more than the real one does (the agent of an agent block need not exist).
"""
import json, os, re, sys

DIR = os.environ["PLATAFORMA_TESTE_DIR"]
TYPES = os.environ["PLATAFORMA_TESTE_TIPOS"].split(",")
# the data each block needs (serde: `missing field`), by block type
REQUIRED = {
    "agent": ["agent_id"], "message": ["content"], "wait": ["seconds"], "condition": ["conditions"], "ask": ["message"],
    "set_variables": ["variables"], "for_each": ["agent_id"], "system_action": ["tool"], "approval": ["question"],
    "notify": ["title"], "subflow": ["flow_id"], "decide": ["questions", "route"], "transcribe": ["file"],
    "describe_image": ["question"], "speak": ["text", "out"], "sandbox": ["template", "entry"],
}
KINDS = ["assistant", "sequential", "parallel", "loop"]


def load(name):
    try:
        return json.load(open(os.path.join(DIR, name)))
    except OSError:
        return []


def save(name, v):
    json.dump(v, open(os.path.join(DIR, name), "w"))


def fail(msg):
    print(json.dumps({"error": {"message": msg}}))
    sys.exit(0)


VAR = re.compile(r"(?:(?:temp|user|app):)?[A-Za-z][A-Za-z0-9_]{0,63}")
RESERVED = ["user_message", "message", "task_instructions", "content", "output", "status", "cycle_count", "session_id", "input"]


def is_variable(name):
    """`variables::is_variable`."""
    if not isinstance(name, str) or not name.strip() or not VAR.fullmatch(name):
        return False
    base = name.rsplit(":", 1)[-1]
    return base not in RESERVED and not base.lower().startswith("kf_")


def key_ok(k):
    return k is None or k == "" or is_variable(k)


def serde_agent(a):
    """What the real `Agent` type refuses while reading (serde), with its words."""
    if not isinstance(a, dict):
        fail("invalid type, expected struct Agent")
    if "id" not in a:
        fail("missing field `id`")
    if not isinstance(a["id"], str):
        fail("invalid type: %s `%s`, expected a string" % ("integer" if isinstance(a["id"], int) else "value", a["id"]))
    if "name" not in a:
        fail("missing field `name`")
    if a.get("kind") not in KINDS:
        fail("unknown variant `%s`, expected one of `assistant`, `sequential`, `parallel`, `loop`" % a.get("kind"))
    cfg = a.get("config")
    if not isinstance(cfg, dict):
        fail("missing field `config`" if cfg is None else "invalid type, expected struct config")
    if a["kind"] == "assistant":
        if "instruction" not in cfg:
            fail("missing field `instruction`")
        for t in cfg.get("tools", []):
            if not isinstance(t, dict) or not isinstance(t.get("name"), str):
                fail("invalid tool: needs a name")
        for s in cfg.get("sub_agents", []):
            if not isinstance(s, dict) or not isinstance(s.get("agent_id"), str):
                fail("invalid sub-agent: needs agent_id")
    else:
        if "sub_agents" not in cfg:
            fail("missing field `sub_agents`")
        if not all(isinstance(s, str) for s in cfg["sub_agents"]):
            fail("invalid type, expected a string (a team's sub-agents are plain ids)")


def agent_issues(a):
    """`Agent::issues`."""
    v = []
    if not a["name"].strip() or len(a["name"]) > 80:
        v.append("the name has 1 to 80 characters")
    cfg = a["config"]
    if not key_ok(cfg.get("output_key")):
        v.append("the result variable must be a variable name")
    if a["kind"] == "assistant":
        if not cfg["instruction"].strip():
            v.append("write the instructions")
        cloud = lambda m: isinstance(m, dict) and m.get("provider") != "local"
        if not a.get("allow_cloud", False) and (cloud(cfg.get("model")) or any(cloud(m) for m in cfg.get("fallback", []))):
            v.append("this agent may not use cloud models")
        sb = cfg.get("sandbox")
        if isinstance(sb, dict) and sb.get("time_limit_s") is not None and not 1 <= sb["time_limit_s"] <= 86400:
            v.append("the machine's time limit goes from 1 second to 1 day")
        for t in cfg.get("tools", []):
            if t.get("permission", "ask") not in ("free", "ask", "never"):
                v.append('unknown permission "%s" for %s' % (t.get("permission"), t["name"]))
        for s in cfg.get("sub_agents", []):
            if s.get("mode", "chat") not in ("chat", "task", "single_turn"):
                v.append('unknown sub-agent mode "%s"' % s.get("mode"))
        refs = [s["agent_id"] for s in cfg.get("sub_agents", [])]
    else:
        if not cfg["sub_agents"]:
            v.append("add at least one sub-agent")
        if a["kind"] == "loop" and not 1 <= cfg.get("max_iterations", 5) <= 20:
            v.append("repeat from 1 to 20 times")
        refs = cfg["sub_agents"]
    if a["id"] in refs:
        v.append("an agent cannot use itself")
    return v


def serde_flow(f):
    """What the real `Flow`/`Block` types refuse while reading, with their words."""
    if not isinstance(f, dict) or "nodes" not in f:
        fail("missing field `nodes`")
    for n in f["nodes"]:
        if "id" not in n:
            fail("missing field `id`")
        if n.get("type") not in TYPES:
            fail("unknown variant `%s`, expected one of %s" % (n.get("type"), ", ".join("`%s`" % t for t in TYPES)))
        # (`end` is a unit variant of the real `Block` enum: a map for its data is refused)
        if n["type"] == "end" and isinstance(n.get("data"), dict):
            fail("invalid type: map, expected unit variant Block::End")
        for k in REQUIRED.get(n["type"], []):
            if k not in (n.get("data") or {}):
                fail("missing field `%s`" % k if n.get("data") is not None else "missing field `data`")
    for e in f.get("edges", []):
        if "source" not in e or "target" not in e:
            fail("missing field `source`")


def flow_issues(f):
    """`Flow::issues`: what keeps a flow from running."""
    out = []
    nodes, edges = f["nodes"], f.get("edges", [])
    if not nodes:
        return ["the flow has no blocks"]
    ids = [n["id"] for n in nodes]
    for i in set(ids):
        if ids.count(i) > 1:
            out.append("two blocks have the same id")
    byid = {n["id"]: n for n in nodes}
    for e in edges:
        if e["source"] not in byid or e["target"] not in byid:
            out.append("a connection points to a block that does not exist (%s -> %s)" % (e["source"], e["target"]))
    for n in nodes:
        d, t = n.get("data") or {}, n["type"]
        if t in ("agent", "for_each") and not str(d.get("agent_id", "")).strip():
            out.append("choose the agent of this block")
        if t == "for_each":
            it = d.get("items", "content").strip()
            if not (it in ("content", "output") or it.startswith("content.") or it.startswith("var:")):
                out.append('items is "content", "content.<field>" or "var:<variable>"')
        if t == "agent" and not key_ok(d.get("save_as")):
            out.append("save_as must be a variable name (letters, digits and _)")
        if t == "for_each":
            if d.get("max_items") is not None and not 1 <= d["max_items"] <= 50:
                out.append("max_items goes from 1 to 50")
            if not key_ok(d.get("save_as")):
                out.append("save_as must be a variable name (letters, digits and _)")
        if t == "set_variables":
            if len(d["variables"]) > 20:
                out.append("at most 20 variables per block")
            for v in d["variables"]:
                if not is_variable(v.get("name")):
                    out.append('"%s" is not a variable name' % v.get("name"))
                if v.get("op") not in ("set", "previous", "increment", "clear"):
                    out.append('unknown operation "%s"' % v.get("op"))
        if t == "condition" and not d["conditions"]:
            out.append("add at least one rule")
        if t == "approval" and not 60 <= d.get("timeout_s", 7200) <= 7 * 24 * 3600:
            out.append("the decision must expire between 1 minute and 7 days")
        if t == "subflow" and not str(d["flow_id"]).strip():
            out.append("choose the flow")
        if t == "sandbox":
            tl = d.get("time_limit_s")
            if tl is not None and not 1 <= tl <= 86400:
                out.append("the machine's time limit goes from 1 second to 1 day")
            if not str(d["template"]).strip():
                out.append("choose the machine template")
            elif d.get("network", "nenhuma") not in ("nenhuma", "internet", "casa"):
                out.append("the network is nenhuma, internet or casa")
            elif d.get("on_end", "apagar") not in ("apagar", "guardar", "se_falhar"):
                out.append("at the end: apagar, guardar or se_falhar")
            elif not any(x["id"] == d["entry"] and x.get("parent") == n["id"] for x in nodes):
                out.append("put at least one block inside the sandbox")
        if t == "decide":
            q = d["questions"]
            if d.get("model", "decisao") not in ("decisao", "decisao-omni"):
                out.append("the decision model is decisao or decisao-omni")
            elif not q:
                out.append("ask at least one question")
            elif d["route"] not in q:
                out.append("the question %s that chooses the way does not exist" % d["route"])
            else:
                for qid, qq in q.items():
                    kind = qq.get("type", "")
                    crit = qq.get("criteria")
                    if kind == "choice" and not (isinstance(crit, dict) and 2 <= len(crit) <= 255):
                        out.append("%s: a choice has 2 to 255 options" % qid)
                    elif kind == "score" and not (isinstance(crit, list) and 2 <= len(crit) <= 10):
                        out.append("%s: a score has 2 to 10 levels" % qid)
                    elif kind not in ("noul", "choice", "score"):
                        out.append("%s: the type is noul, choice or score" % qid)
                    elif qq.get("instructions") is None:
                        out.append("%s: write the question" % qid)
                if not 0 <= d.get("threshold", 0.8) <= 1:
                    out.append("the confidence goes from 0 to 1")
    # blocks no run reaches (the entry is the first top-level block nothing leads to)
    targets = {e["target"] for e in edges}
    top = [n for n in nodes if not n.get("parent")]
    entry = next((n for n in top if n["id"] not in targets), top[0] if top else None)
    if entry:
        reach, queue = {entry["id"]}, [entry["id"]]
        while queue:
            cur = queue.pop(0)
            nxt = [e["target"] for e in edges if e["source"] == cur]
            if byid.get(cur, {}).get("type") == "sandbox":
                nxt.append(byid[cur]["data"]["entry"])
            for x in nxt:
                if x not in reach:
                    reach.add(x)
                    queue.append(x)
        for n in nodes:
            if n["id"] not in reach:
                out.append("no connection reaches this block")
    return out


def clamp(a):
    """What the real preview returns for an agent: the `Agent` as it serialises (defaults filled) fitted to the creator's
    ceiling. The ceiling here is the tools named in $PLATAFORMA_TESTE_NEGADAS, which the creator may not give."""
    a = json.loads(json.dumps(a))
    a.setdefault("description", "")
    cfg = a["config"]
    notes = []
    denied = [t for t in os.environ.get("PLATAFORMA_TESTE_NEGADAS", "").split(",") if t]
    tools = []
    for t in cfg.get("tools", []):
        t["name"] = t["name"].replace("__", ".")
        t.setdefault("permission", "ask")
        if t["name"] in denied:
            notes.append("removed:" + t["name"])
        else:
            tools.append(t)
    if "tools" in cfg:
        cfg["tools"] = tools
    for s in cfg.get("sub_agents", []):
        if isinstance(s, dict):
            s.setdefault("mode", "chat")
    return a, notes


def main():
    method, params = sys.argv[2], json.loads(sys.argv[3])
    with open(os.path.join(DIR, "chamadas.jsonl"), "a") as log:
        log.write(json.dumps({"method": method, "params": params}) + "\n")
    agents, flows = load("agents.json"), load("flows.json")
    if method == "agents.list":
        print(json.dumps({"result": [{"agent": a, "created_by": "agent:teste"} for a in agents]}))
        return
    if method == "agents.get":
        found = [a for a in agents if a["id"] == params["id"]]
        if not found:
            fail("not found")
        print(json.dumps({"result": found[0]}))
        return
    if method == "flows.get":
        found = [f for f in flows if f["id"] == params["id"]]
        if not found:
            fail("not found")
        print(json.dumps({"result": found[0]}))
        return
    if method == "flows.run":
        print(json.dumps({"result": {"output": "(test double: nothing ran)"}}))
        return
    if method not in ("platform.preview", "platform.apply"):
        fail("unknown method " + method)
    kind, item = params["kind"], params["item"]
    if kind == "agent":
        serde_agent(item)
        problems = agent_issues(item)
        if not item["id"] or (method == "platform.apply" and any(a["id"] == item["id"] for a in agents) and not params.get("edit")):
            item["id"] = "gerado-%d" % (len(agents) + 1)
    elif kind == "flow":
        serde_flow(item["flow"])
        problems = flow_issues(item["flow"])
    else:
        fail("unknown kind " + kind)
    notes = []
    if kind == "agent":
        item, notes = clamp(item)
    if method == "platform.preview":
        print(json.dumps({"result": {"item": item, "notes": notes, "problems": problems}}))
        return
    if problems:
        fail("; ".join(problems))
    if kind == "agent":
        agents = [a for a in agents if a["id"] != item["id"]] + [item]
        save("agents.json", agents)
        ident = item["id"]
    else:
        ident = item.get("id") or "fluxo-%d" % (len(flows) + 1)
        flows.append({"id": ident, "name": item.get("name"), "flow": item["flow"]})
        save("flows.json", flows)
    print(json.dumps({"result": {"id": ident, "notes": notes}}))


main()
