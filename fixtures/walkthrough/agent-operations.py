#!/usr/bin/env python3
"""Live HTTP/MCP walkthrough, not a unit-test runner. Use only the disposable
database and private credentials provisioned by the agent-operations binary.
Every mutation goes through the real wire edge; SQL observes its effects.
Tokens and plan codes never appear in output. See the ledger README.
"""
import json
import os
import subprocess
import urllib.error
import urllib.request

if os.environ.get("BILIBI_AGENT_PROOF") != "disposable":
    raise SystemExit("BILIBI_AGENT_PROOF=disposable is required")
BASE = os.environ.get("BILIBI_AGENT_URL", "http://127.0.0.1:3100").rstrip("/")
CREDS = json.load(open(os.environ["BILIBI_AGENT_CREDENTIALS"]))
DB = os.environ["DATABASE_URL"]


def sql(query):
    return subprocess.check_output(
        ["psql", DB, "-XAt", "-v", "ON_ERROR_STOP=1", "-c", query], text=True
    ).strip()


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


require(sql("SELECT count(*) FROM users WHERE id=1 AND email='manager@example.test'") == "1", "not the disposable fixture")


def request(path, body=None, who="manager", channel="api"):
    headers = {"Authorization": "Bearer " + CREDS[who][channel], "Content-Type": "application/json"}
    if channel == "mcp":
        headers["Accept"] = "application/json, text/event-stream"
    req = urllib.request.Request(BASE + path, data=None if body is None else json.dumps(body).encode(), headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=30) as response:
            return response.status, json.load(response)
    except urllib.error.HTTPError as response:
        return response.code, json.load(response)


def invoke(channel, tool, args, who="manager"):
    if channel == "mcp":
        status, body = request("/mcp", {"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": tool, "arguments": args}}, who, channel)
        require(status == 200, f"MCP transport {status}")
        require("result" in body, "MCP protocol refusal")
        return body["result"]["structuredContent"]
    paths = {"plan": "/api/v1/plans", "commit": "/api/v1/plans/commit"}
    if tool == "read":
        path = "/api/v1/operations/" + args["key"] + "/call"
        args = {"input": args["input"]}
    elif tool in ("guide", "describe"):
        segment = "guides" if tool == "guide" else "operations"
        return request("/api/v1/" + segment + "/" + args["key"], who=who)[1]
    else:
        path = paths[tool]
    return request(path, args, who)[1]


def operation(key, company, **values):
    return {"key": "finance.ledger." + key, "input": {"company_id": company, **values}}


def read(channel, key, company, who="manager", **values):
    op = operation(key, company, **values)
    body = invoke(channel, "read", op, who)
    require("result" in body, f"{key}: read failed: {body.get('error')}")
    return body["result"]


def plan(channel, ops, who="manager"):
    return invoke(channel, "plan", {"operations": ops}, who)


def commit(channel, ops, review, who="manager"):
    return invoke(channel, "commit", {"operations": ops, "plan_code": review["plan_code"]}, who)


def write(channel, key, company, who="manager", **values):
    ops = [operation(key, company, **values)]
    review = plan(channel, ops, who)
    require(review.get("valid") is True, f"{key}: preview refused: {review.get('results', review.get('error'))}")
    body = commit(channel, ops, review, who)
    require("results" in body, f"{key}: commit refused: {body.get('error')}")
    return body["results"][0]["result"]["result"]["record"]


def refused(channel, key, company, contains, who="manager", **values):
    body = plan(channel, [operation(key, company, **values)], who)
    require(body.get("valid") is False and "plan_code" not in body and contains in json.dumps(body), f"{key}: expected refusal {contains}")


for channel, company in [("api", 1), ("mcp", 2)]:
    print(f"\n== {channel}: company {company}")
    require(read(channel, "book.get", company)["book"] is None, "proof needs an unopened book")
    require(invoke(channel, "guide", {"key": "finance.ledger"}), "guide missing")
    require(invoke(channel, "describe", {"key": "finance.ledger.journal.submit"}), "schema missing")
    direct = invoke(channel, "read", operation("book.open", company, currency_code="MYR", minor_units=2))
    require(direct.get("error") == "writes_not_enabled", "direct write was not refused")
    ops = [operation("book.open", company, currency_code="myr", minor_units=2)]
    preview = plan(channel, ops)
    require(preview.get("valid") is True, "book preview failed")
    require(sql(f"SELECT count(*) FROM finance_ledger_books WHERE company_id={company}") == "0", "preview wrote a book")
    require("results" in commit(channel, ops, preview), "book commit failed")
    require(commit(channel, ops, preview).get("error") == "plan_already_used", "code replay accepted")
    require(read(channel, "book.get", company)["book"]["currency_code"] == "MYR", "currency not normalized")
    print("book preview wrote no business rows; commit normalized currency; direct write and code replay refused")

    asset = write(channel, "account.create", company, code="A", name="Asset", account_type="asset")
    income = write(channel, "account.create", company, code="I", name="Income", account_type="income")
    retained = write(channel, "account.create", company, code="E", name="Equity", account_type="equity", retained_earnings=True)
    child = write(channel, "account.create", company, code="B", name="Child", account_type="asset", parent_id=asset["id"])
    child = write(channel, "account.update", company, account_id=child["id"], name="Changed", clear_parent=True, active=False)
    require(child["parent_id"] is None and child["active"] is False, "account update lost fields")
    page = read(channel, "account.list", company, page_size=2)
    page2 = read(channel, "account.list", company, page_size=2, after_code=page["next_after_code"])
    require([a["code"] for a in page["records"] + page2["records"]] == ["A", "B", "E", "I"], "account cursor skipped/duplicated rows")
    # Locale-aware PostgreSQL ordering must also govern the cursor predicate.
    # The original Rust string comparison skipped B after a in en_US.UTF-8.
    for code in ["a", "É", "z"]:
        write(channel, "account.create", company, code=code, name=code, account_type="asset")
    expected_codes = json.loads(sql(f"SELECT json_agg(code ORDER BY code) FROM finance_ledger_accounts WHERE company_id={company}"))
    codes, cursor = [], None
    while True:
        page = read(channel, "account.list", company, page_size=1, **({"after_code": cursor} if cursor else {}))
        codes.extend(a["code"] for a in page["records"])
        cursor = page["next_after_code"]
        if cursor is None:
            break
        require(len(codes) <= len(expected_codes), "account cursor did not advance")
    require(codes == expected_codes, "mixed-case/non-ASCII account cursor disagrees with database order")
    year = write(channel, "fiscal_year.open", company, start_date="2026-01-01", end_date="2026-02-28")
    first, last = year["periods"]
    print("created and changed accounts; exact two-page chart; opened two monthly periods")

    lines = [{"account_id": asset["id"], "debit": "101.23"}, {"account_id": income["id"], "credit": "101.23"}]
    refused(channel, "journal.submit", company, "decimal places", entry_date="2026-01-19", description="Precision", lines=[{**lines[0], "debit": "101.231"}, lines[1]])
    refused(channel, "journal.submit", company, "do not balance", entry_date="2026-01-19", description="Unbalanced", lines=[{**lines[0], "debit": "101.22"}, lines[1]])
    journal = write(channel, "journal.submit", company, entry_date="2026-01-19", description="Manual adjustment", lines=lines)
    require(journal["status"] == "submitted" and read(channel, "entry.list", company)["records"] == [], "submission posted")
    refused(channel, "journal.approve", company, "cannot approve", journal_id=journal["id"])
    approved = write(channel, "journal.approve", company, who="approver", journal_id=journal["id"])
    require(approved["status"] == "approved", "independent approval failed")
    entry_id = approved["decision"]["entry_id"]
    entry = read(channel, "entry.get", company, entry_id=entry_id)
    require(entry["lines"][0]["debit"] == "101.23" and entry["lines"][1]["credit"] == "101.23", "posted amounts differ")
    require(read(channel, "journal.get", company, journal_id=journal["id"])["decision"]["entry_id"] == entry_id, "read lost decision")
    rejected = write(channel, "journal.submit", company, entry_date="2026-01-20", description="Reject this", lines=lines)
    rejected = write(channel, "journal.reject", company, journal_id=rejected["id"], reason="Superseded")
    require(rejected["status"] == "rejected" and rejected["decision"]["entry_id"] is None, "rejection posted")
    reversed_entry = write(channel, "entry.reverse", company, entry_id=entry_id, entry_date="2026-01-21", description="Correction")
    require(reversed_entry["reverses_entry_id"] == entry_id and reversed_entry["lines"][0]["credit"] == "101.23", "reversal is not the mirror")
    refused(channel, "entry.reverse", company, "already reversed", entry_id=entry_id, entry_date="2026-01-22", description="Twice")
    print("precision/balance refusals; submit without posting; same-person refusal; independent posting; rejection; exact mirror and double-reversal refusal")

    # Keep nonzero income so close/reopen must show actual generated entries.
    posted = write(channel, "journal.submit", company, entry_date="2026-02-17", description="Year activity", lines=lines)
    write(channel, "journal.approve", company, who="approver", journal_id=posted["id"])
    pending = write(channel, "journal.submit", company, entry_date="2026-01-25", description="Period will close", lines=lines)
    write(channel, "period.close", company, period_id=first["id"], reason="Complete")
    refused(channel, "journal.approve", company, "closed period", who="approver", journal_id=pending["id"])
    write(channel, "period.reopen", company, period_id=first["id"], reason="Correction window")
    write(channel, "period.close", company, period_id=first["id"])
    write(channel, "period.lock", company, period_id=first["id"])
    refused(channel, "period.reopen", company, "locked", period_id=first["id"], reason="Never")
    closing_ops = [operation("fiscal_year.close", company, fiscal_year_id=year["id"])]
    closing_review = plan(channel, closing_ops)
    require(closing_review.get("valid") is True and "101.23" in json.dumps(closing_review), "close review missing generated amounts")
    closed = commit(channel, closing_ops, closing_review)["results"][0]["result"]["result"]["record"]
    require(closed["status"] == "closed" and closed["closing_entry_id"] is not None, "year did not post closing entry")
    close_entry = read(channel, "entry.get", company, entry_id=closed["closing_entry_id"])
    require(any(l["account_id"] == retained["id"] and l["credit"] == "101.23" for l in close_entry["lines"]), "wrong retained-earnings side")
    reopened = write(channel, "fiscal_year.reopen", company, fiscal_year_id=year["id"], reason="Year correction")
    require(reopened["status"] == "open" and reopened["periods"][0]["status"] == "locked" and reopened["periods"][-1]["status"] == "open", "reopen changed other periods")
    require(read(channel, "entry.get", company, entry_id=close_entry["id"])["reversed_by_entry_id"] is not None, "reopen did not reverse close")
    write(channel, "fiscal_year.close", company, fiscal_year_id=year["id"])
    locked = write(channel, "fiscal_year.lock", company, fiscal_year_id=year["id"], reason="Final")
    require(all(p["status"] == "locked" for p in locked["periods"]), "year lock missed periods")
    refused(channel, "fiscal_year.reopen", company, "locked", fiscal_year_id=year["id"], reason="Never")
    print("period close/reopen/permanent lock; closed-period approval refusal; year close generated exact retained-earnings entry; reopen reversed it; whole-year permanent lock")

    # Both previews default to 2026-03-01. A naive implementation would open
    # two different years. The owner fence must roll back the FIRST insert too.
    doubles = [operation("fiscal_year.open", company), operation("fiscal_year.open", company)]
    review = plan(channel, doubles)
    require(review.get("valid") is True, "composition setup preview failed")
    before = sql(f"SELECT count(*) FROM finance_ledger_fiscal_years WHERE company_id={company}")
    used = sql("SELECT count(*) FROM base_agent_api_plans WHERE used_at IS NOT NULL")
    body = commit(channel, doubles, review)
    require(body.get("error") == "stale_plan", "within-plan default drift was not fenced")
    require(sql(f"SELECT count(*) FROM finance_ledger_fiscal_years WHERE company_id={company}") == before, "partial first operation survived rollback")
    require(sql("SELECT count(*) FROM base_agent_api_plans WHERE used_at IS NOT NULL") == used, "failed code was consumed")
    require(commit(channel, doubles, review).get("error") == "stale_plan", "failed code cannot retry unchanged state")
    require("results" in commit(channel, [operation("fiscal_year.open", company)], plan(channel, [operation("fiscal_year.open", company)])), "fresh plan after rollback failed")
    require(int(sql("SELECT count(*) FROM base_agent_api_plans WHERE used_at IS NOT NULL")) == int(used) + 1, "failed code was consumed")
    print("two defaulted years refused as stale; first insert rolled back; failed code unconsumed; fresh plan succeeds")

    foreign = invoke(channel, "read", operation("book.get", company), "other")
    require("error" in foreign and "result" not in foreign, "cross-tenant disclosure")
    extra = invoke(channel, "read", operation("book.get", company, actor_id=1))
    require(extra.get("error") == "invalid_input", "caller-supplied actor accepted")
    print("cross-tenant read and caller-supplied actor refused")

print("\nHTTP and MCP ledger walkthrough completed; all observed records matched the reviewed contracts.")
