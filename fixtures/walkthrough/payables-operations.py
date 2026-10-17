#!/usr/bin/env python3
"""H3 running-app proof through real OAuth HTTP and resource-bound MCP.
No mocks or unit-test runner. SQL observes; every business write uses a facade
through the wire.
"""
import json
import os
import subprocess
import urllib.error
import urllib.request
from decimal import Decimal

if os.environ.get("BILIBI_AGENT_PROOF") != "disposable":
    raise SystemExit("BILIBI_AGENT_PROOF=disposable is required")
BASE = os.environ.get("BILIBI_AGENT_URL", "http://127.0.0.1:3100").rstrip("/")
CREDS = json.load(open(os.environ["BILIBI_AGENT_CREDENTIALS"]))
DB = os.environ["DATABASE_URL"]


def require(ok, message):
    if not ok:
        raise RuntimeError(message)


def sql(query):
    return subprocess.check_output(["psql", DB, "-XAt", "-v", "ON_ERROR_STOP=1", "-c", query], text=True).strip()


require(sql("SELECT count(*) FROM users WHERE id=1 AND email='manager@example.test'") == "1", "not the disposable fixture")


def request(path, data=None, who="manager", channel="api"):
    headers = {"Authorization": "Bearer " + CREDS[who][channel], "Content-Type": "application/json"}
    if channel == "mcp":
        headers["Accept"] = "application/json, text/event-stream"
    req = urllib.request.Request(BASE + path, data=None if data is None else json.dumps(data).encode(), headers=headers)
    try:
        response = urllib.request.urlopen(req, timeout=30)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        return json.load(response)


def invoke(channel, tool, args, who="manager"):
    if channel == "mcp":
        body = request("/mcp", {"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": tool, "arguments": args}}, who, channel)
        require("result" in body, "MCP protocol refusal")
        return body["result"]["structuredContent"]
    if tool in ("describe", "guide"):
        return request("/api/v1/" + ("guides/" if tool == "guide" else "operations/") + args["key"], who=who)
    path = {"plan": "/api/v1/plans", "commit": "/api/v1/plans/commit"}.get(tool)
    if tool == "read":
        path = "/api/v1/operations/" + args["key"] + "/call"
        args = {"input": args["input"]}
    return request(path, args, who)


def op(key, company, **values):
    if not key.startswith("finance."):
        key = "finance.payables." + key
    return {"key": key, "input": {"company_id": company, **values}}


def read(channel, key, company, who="manager", **values):
    body = invoke(channel, "read", op(key, company, **values), who)
    require("result" in body, f"{key}: read refused: {body.get('error')}")
    return body["result"]


def plan(channel, ops, who="manager"):
    return invoke(channel, "plan", {"operations": ops}, who)


def commit(channel, ops, review, who="manager"):
    return invoke(channel, "commit", {"operations": ops, "plan_code": review["plan_code"]}, who)


def record(body):
    require("results" in body, f"commit refused: {body.get('error')}")
    return body["results"][0]["result"]["result"]["record"]


def write(channel, key, company, who="manager", **values):
    ops = [op(key, company, **values)]
    review = plan(channel, ops, who)
    require(review.get("valid") is True, f"{key}: preview refused: {review.get('results', review.get('error'))}")
    return record(commit(channel, ops, review, who))


def refused(channel, key, company, contains, who="manager", **values):
    review = plan(channel, [op(key, company, **values)], who)
    require(review.get("valid") is False and "plan_code" not in review and contains in json.dumps(review), f"{key}: expected refusal {contains}, got {review.get('results', review.get('error'))}")


def changes(review):
    return review["results"][0]["preview"]["result"]["changes"]


def entry_lines(entry):
    return [(l["account_id"], Decimal(l["debit"]), Decimal(l["credit"]), l["description"]) for l in entry["lines"]]


def counters(company):
    return sql(f"SELECT (SELECT count(*) FROM finance_payables_settings_changes WHERE company_id={company}), (SELECT count(*) FROM finance_ledger_entries WHERE company_id={company}), (SELECT count(*) FROM base_audit_mutations), (SELECT count(*) FROM base_agent_api_plans WHERE used_at IS NOT NULL)")


for channel, company in [("api", 1), ("mcp", 2)]:
    print(f"\n== {channel}: company {company}", flush=True)
    require(read(channel, "settings.get", company)["settings"] is None, "needs fresh database")
    require(invoke(channel, "guide", {"key": "finance.payables"}), "guide missing")
    require(invoke(channel, "describe", {"key": "finance.payables.bill.create"}), "schema missing")
    require(invoke(channel, "read", op("settings.put", company)).get("error") == "writes_not_enabled", "direct write accepted")
    write(channel, "finance.ledger.book.open", company, currency_code="MYR", minor_units=2)
    accounts = {}
    for code, kind in [("A", "asset"), ("L", "liability"), ("M", "liability"), ("T", "asset"), ("X", "expense")]:
        accounts[code] = write(channel, "finance.ledger.account.create", company, code=code, name=code, account_type=kind)["id"]
    year = write(channel, "finance.ledger.fiscal_year.open", company, start_date="2026-01-01", end_date="2026-03-31")
    write(channel, "settings.put", company, payable_account_id=accounts["L"], quantity_tolerance={"kind": "percent", "value": "2.5"}, price_tolerance={"kind": "amount", "value": "0.01"})
    original = read(channel, "settings.get", company)["settings"]
    before = sql(f"SELECT count(*) FROM finance_payables_settings_changes WHERE company_id={company}")
    write(channel, "settings.put", company, reason="No change")
    require(read(channel, "settings.get", company)["settings"] == original, "omission did not preserve settings")
    require(sql(f"SELECT count(*) FROM finance_payables_settings_changes WHERE company_id={company}") == before, "no-op made history")
    write(channel, "settings.put", company, clear_quantity_tolerance=True)
    settings = read(channel, "settings.get", company)["settings"]
    require(settings["quantity_tolerance"] is None and settings["price_tolerance"] == original["price_tolerance"], "clear lost unrelated value")
    refused(channel, "settings.put", company, "together", clear_price_tolerance=True, price_tolerance={"kind": "amount", "value": "1"})
    tax = write(channel, "tax_code.create", company, code="INPUT", name="Input", rate="5", account_id=accounts["T"])
    before = sql(f"SELECT count(*) FROM finance_payables_tax_code_changes WHERE company_id={company}")
    same = write(channel, "tax_code.update", company, tax_code_id=tax["id"])
    require(Decimal(same["rate"]) == Decimal(tax["rate"]) == 5 and {k: v for k, v in same.items() if k != "rate"} == {k: v for k, v in tax.items() if k != "rate"} and sql(f"SELECT count(*) FROM finance_payables_tax_code_changes WHERE company_id={company}") == before, "tax no-op lost values or made history")
    print("settings omissions, explicit clears, conflicting clear refusal; tax create/update no-op and immutable code", flush=True)

    draft = {"party_key": "party-a", "party_name": "Party A", "party_document_number": "SOURCE-1", "document_date": "2026-01-10", "lines": [
        {"description": "Net", "account_id": accounts["X"], "amount": "100.05", "tax_code_id": tax["id"]},
        {"description": "Fraction", "account_id": accounts["X"], "amount": "0.05", "tax_code_id": tax["id"]},
    ]}
    refused(channel, "bill.create", company, "lines.0.amount", **{**draft, "lines": [{**draft["lines"][0], "amount": "100.051"}]})
    refused(channel, "bill.create", company, "actor_id", **{**draft, "actor_id": 1})
    ops = [op("bill.create", company, **draft)]
    sequence = sql("SELECT last_value,is_called FROM finance_payables_documents_id_seq")
    review = plan(channel, ops)
    require(review.get("valid") is True, "bill preview refused")
    require(sql(f"SELECT count(*) FROM finance_payables_documents WHERE company_id={company}") == "0", "preview wrote a document")
    require(sql("SELECT last_value,is_called FROM finance_payables_documents_id_seq") == sequence, "preview consumed sequence")
    require(changes(review)["document"]["total_tax"] == "5.01", "tax did not round once per group")
    bill = record(commit(channel, ops, review))
    require(commit(channel, ops, review).get("error") == "plan_already_used", "successful code replay accepted")
    require(bill["status"] == "draft" and bill["entry_id"] is None and bill["total"] == "105.11" and bill["due_date"] == "2026-01-10", "draft snapshots differ")
    refused(channel, "bill.create", company, "already recorded", **draft)
    write(channel, "tax_code.update", company, tax_code_id=tax["id"], rate="9", active=False)
    require(Decimal(read(channel, "document.get", company, document_id=bill["id"])["tax_lines"][0]["rate"]) == 5, "tax snapshot changed")
    refused(channel, "document.approve", company, "cannot approve", document_id=bill["id"])
    write(channel, "document.approve", company, who="approver", document_id=bill["id"])
    ops = [op("document.post", company, document_id=bill["id"])]
    review = plan(channel, ops)
    expected = changes(review)["posting"]["entry"]
    bill = record(commit(channel, ops, review))
    entry = read(channel, "finance.ledger.entry.get", company, entry_id=bill["entry_id"])
    require(entry_lines(entry) == entry_lines(expected), "posted lines differ from review")
    require([(d, c) for _, d, c, _ in entry_lines(entry)] == [(Decimal("100.05"), 0), (Decimal("0.05"), 0), (Decimal("5.01"), 0), (0, Decimal("105.11"))], "independent bill arithmetic differs")
    require(entry["owner"]["record"] == f"document:{bill['id']}", "bill not owned")
    print("no-write/no-sequence preview; exact grouped 5.01 tax on 100.10 net; independent approval; reviewed posting equals ledger; replay, duplicate and precision refused", flush=True)

    note = write(channel, "debit_note.create", company, bill_id=bill["id"], document_date="2026-01-11", lines=[{"description": "Credit", "account_id": accounts["X"], "amount": "10.11"}])
    require(note["party"] == bill["party"], "note failed to copy party")
    write(channel, "document.approve", company, who="approver", document_id=note["id"])
    write(channel, "settings.put", company, payable_account_id=accounts["M"])
    note = write(channel, "document.post", company, document_id=note["id"])
    note_entry = read(channel, "finance.ledger.entry.get", company, entry_id=note["entry_id"])
    require([(a, d, c) for a, d, c, _ in entry_lines(note_entry)] == [(accounts["X"], 0, Decimal("10.11")), (accounts["L"], Decimal("10.11"), 0)], "note used new control or wrong direction")
    extra = write(channel, "bill.create", company, **{**draft, "party_document_number": "SOURCE-2", "document_date": "2026-01-12", "lines": [{"description": "Other", "account_id": accounts["X"], "amount": "20"}]})
    write(channel, "document.approve", company, who="approver", document_id=extra["id"])
    extra = write(channel, "document.post", company, document_id=extra["id"])
    void = write(channel, "bill.create", company, **{**draft, "party_document_number": "VOID", "lines": [{"description": "Void", "account_id": accounts["X"], "amount": "1"}]})
    void = write(channel, "document.void", company, document_id=void["id"], void_reason="Duplicate draft")
    require(void["status"] == "voided" and void["void_reason"] == "Duplicate draft", "void lost reason")

    payment = {"payment_date": "2026-01-20", "party_key": "party-a", "paid_from_account_id": accounts["A"], "allocations": [{"document_id": bill["id"], "amount": "40"}, {"document_id": extra["id"], "amount": "7.25"}]}
    request_record = write(channel, "payment.submit", company, **payment)
    require(request_record["status"] == "submitted" and read(channel, "payment.list", company)["records"] == [], "submission posted")
    refused(channel, "payment.approve", company, "cannot approve", request_id=request_record["id"], override_reason="Cannot bypass separation")
    refused(channel, "payment.submit", company, "allocations.0.amount", **{**payment, "allocations": [{"document_id": bill["id"], "amount": "95.001"}]})
    ops = [op("payment.approve", company, request_id=request_record["id"])]
    sequence = sql("SELECT last_value,is_called FROM finance_payables_payments_id_seq")
    review = plan(channel, ops, "approver")
    require(review.get("valid") is True and sql("SELECT last_value,is_called FROM finance_payables_payments_id_seq") == sequence, "payment preview consumed ID")
    expected = changes(review)["posting"]["entry"]["entry"]
    paid = record(commit(channel, ops, review, "approver"))
    paid_entry = read(channel, "finance.ledger.entry.get", company, entry_id=paid["entry_id"])
    require(entry_lines(paid_entry) == entry_lines(expected), "payment differs from review")
    amounts = {a: (d, c) for a, d, c, _ in entry_lines(paid_entry)}
    require(amounts == {accounts["L"]: (Decimal("40"), 0), accounts["M"]: (Decimal("7.25"), 0), accounts["A"]: (0, Decimal("47.25"))}, "payment netted historical controls")
    require(paid_entry["owner"]["record"] == f"payment:{paid['id']}", "payment owned by request instead")
    require(read(channel, "payment_request.get", company, request_id=request_record["id"])["decision"]["payment_id"] == paid["id"], "request lost decision")
    rejected = write(channel, "payment.submit", company, **{**payment, "allocations": [{"document_id": extra["id"], "amount": "1"}]})
    rejected = write(channel, "payment.reject", company, request_id=rejected["id"], reason="Superseded")
    require(rejected["status"] == "rejected" and rejected["decision"]["payment_id"] is None, "reject posted")
    # The ledger only deactivates zero-balance accounts. A real independently
    # approved reclassification clears them before the settlement reversal.
    clearing = write(channel, "finance.ledger.journal.submit", company, entry_date="2026-01-31", description="Reclassify balances before account retirement", lines=[
        {"account_id": accounts["A"], "debit": "47.25"},
        {"account_id": accounts["L"], "debit": "55"},
        {"account_id": accounts["M"], "debit": "12.75"},
        {"account_id": accounts["X"], "credit": "115"},
    ])
    write(channel, "finance.ledger.journal.approve", company, who="approver", journal_id=clearing["id"])
    for code in ["A", "L", "M"]:
        write(channel, "finance.ledger.account.update", company, account_id=accounts[code], active=False)
    reversed_payment = write(channel, "payment.reverse", company, payment_id=paid["id"], payment_date="2026-02-01")
    reversed_entry = read(channel, "finance.ledger.entry.get", company, entry_id=reversed_payment["entry_id"])
    require([(a, d, c) for a, d, c, _ in entry_lines(reversed_entry)] == [(a, c, d) for a, d, c, _ in entry_lines(paid_entry)], "reversal is not exact mirror")
    require(reversed_payment["allocations"] == [] and read(channel, "document.get", company, document_id=bill["id"])["outstanding"] == "95", "reversal failed to restore outstanding")
    refused(channel, "payment.reverse", company, "already reversed", payment_id=paid["id"], payment_date="2026-02-02")
    refused(channel, "payment.reverse", company, "cannot be reversed", payment_id=reversed_payment["id"], payment_date="2026-02-02")
    for code in ["A", "L", "M"]:
        write(channel, "finance.ledger.account.update", company, account_id=accounts[code], active=True)
    print("note retains old control; mixed-control payment 40 + 7.25 = 47.25; request decisions, owned mirror after deactivation and dated outstanding; double/reversal-of-reversal refused", flush=True)

    # A later payment settled 40 until its February reversal. A January note
    # cannot use today's 95 when the future minimum is only 55.
    refused(channel, "debit_note.create", company, "outstanding", bill_id=bill["id"], document_date="2026-01-15", lines=[{"description": "Too much in January", "account_id": accounts["X"], "amount": "55.01"}])
    refused(channel, "payment.submit", company, "exceeds what is owed", **{**payment, "payment_date": "2026-01-15", "allocations": [{"document_id": bill["id"], "amount": "55.01"}]})
    later = write(channel, "bill.create", company, **{**draft, "party_document_number": "LATER", "lines": [{"description": "Later", "account_id": accounts["X"], "amount": "12.34"}]})
    write(channel, "document.approve", company, who="approver", document_id=later["id"])
    ops = [op("settings.put", company, payable_account_id=accounts["L"]), op("document.post", company, document_id=later["id"])]
    review = plan(channel, ops)
    require(review.get("valid") is True, "same-plan setup failed")
    before = counters(company)
    require(commit(channel, ops, review).get("error") == "stale_plan", "earlier setting change escaped review fence")
    require(counters(company) == before, "settings/history/entry/audit/code survived rollback")
    require(commit(channel, ops, review).get("error") == "stale_plan", "failed code consumed")
    require(read(channel, "settings.get", company)["settings"]["payable_account_id"] == accounts["M"], "settings survived failed plan")
    write(channel, "document.post", company, document_id=later["id"])
    # Same-plan payment approvals: capacity still sufficient for the second,
    # but the review changed, so it must roll back even without overdrawing.
    a = write(channel, "payment.submit", company, **{**payment, "payment_date": "2026-02-05", "allocations": [{"document_id": bill["id"], "amount": "10"}]})
    b = write(channel, "payment.submit", company, **{**payment, "payment_date": "2026-02-05", "allocations": [{"document_id": bill["id"], "amount": "11"}]})
    ops = [op("payment.approve", company, request_id=r["id"]) for r in [a, b]]
    review = plan(channel, ops, "approver")
    require(review.get("valid") is True, "competing approval previews failed")
    before = counters(company)
    require(commit(channel, ops, review, "approver").get("error") == "stale_plan", "capacity drift accepted")
    require(counters(company) == before and read(channel, "payment_request.get", company, request_id=a["id"])["status"] == "submitted", "first payment/audit/code escaped rollback")
    print("future minimum, not current balance, enforced; changed control and competing capacity plans roll back every row/audit/code; failed codes remain usable", flush=True)

    for key, table in [("tax_code.list", "tax_codes"), ("document.list", "documents"), ("payment_request.list", "payment_requests"), ("payment.list", "payments")]:
        ids, cursor = [], 0
        while True:
            page = read(channel, key, company, page_size=1, after_id=cursor)
            ids.extend(r["id"] for r in page["records"])
            cursor = page["next_after_id"]
            if cursor is None:
                break
            require(len(ids) < 50, "cursor did not advance")
        expected = json.loads(sql(f"SELECT json_agg(id ORDER BY id) FROM finance_payables_{table} WHERE company_id={company}"))
        require(ids == expected, f"{key} cursor lost records")
    require(len(read(channel, "document.list", company, status="voided")["records"]) == 1, "document status filter failed")
    require(len(read(channel, "payment_request.list", company, status="rejected")["records"]) == 1, "request status filter failed")
    require(read(channel, "payment.list", company, party_key="absent")["records"] == [], "party filter ignored")
    require("error" in invoke(channel, "read", op("document.get", company, document_id=bill["id"]), "other"), "cross-tenant disclosure")
    write(channel, "finance.ledger.period.close", company, period_id=year["periods"][0]["id"])
    pending = write(channel, "bill.create", company, **{**draft, "party_document_number": "CLOSED", "lines": [{"description": "Closed period", "account_id": accounts["X"], "amount": "2"}]})
    write(channel, "document.approve", company, who="approver", document_id=pending["id"])
    refused(channel, "document.post", company, "closed period", document_id=pending["id"])
    print("every read and cursor exercised; filters, cross-tenant denial, closed-period posting refusal; draft creation/approval remain available in closed period", flush=True)

print("\nHTTP and MCP Payables walkthrough completed; all observed records matched independently expected effects.", flush=True)
