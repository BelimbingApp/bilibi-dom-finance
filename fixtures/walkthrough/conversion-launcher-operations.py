#!/usr/bin/env python3
"""Compare the launcher browser's two-account opening reviews with real MCP.

Run `preview` before approving in the browser, then `verify` after both imports.
Use only the disposable database and private files made by conversion-launcher.
"""
import json
import os
import subprocess
import sys
import urllib.parse
import urllib.request
from decimal import Decimal
from pathlib import Path

if os.environ.get("BILIBI_AGENT_PROOF") != "disposable":
    raise SystemExit("BILIBI_AGENT_PROOF=disposable is required")
fixture = json.loads(Path(os.environ["CONVERSION_FIXTURE_DATA"]).read_text())
credential = json.loads(Path(os.environ["BILIBI_AGENT_CREDENTIALS"]).read_text())["approver"]["mcp"]
evidence = Path(os.environ["CONVERSION_EVIDENCE"])
base = os.environ.get("BILIBI_AGENT_URL", "http://127.0.0.1:18762").rstrip("/")


def call(tool, arguments):
    body = {"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": tool, "arguments": arguments}}
    request = urllib.request.Request(base + "/mcp", data=json.dumps(body).encode(), headers={"Authorization": "Bearer " + credential, "Content-Type": "application/json", "Accept": "application/json, text/event-stream"})
    with urllib.request.urlopen(request, timeout=30) as response:
        message = json.load(response)
    if "result" not in message:
        raise RuntimeError("MCP protocol refused the call")
    return message["result"]["structuredContent"]


def source(module):
    values = {"company_id": fixture["company_id"], "source_reference": "AR-SOURCE" if module == "receivables" else "AP-SOURCE", "currency_code": "MYR", "csv": fixture["ar_csv" if module == "receivables" else "ap_csv"]}
    values["historical_control_account_codes"] = fixture["ar_historical_control_account_codes" if module == "receivables" else "ap_historical_control_account_codes"]
    if module == "receivables":
        values["series_id"] = fixture["series_id"]
    return {"key": f"finance.{module}.opening.import", "input": values}


def sql(query):
    address = urllib.parse.urlsplit(os.environ["DATABASE_URL"])
    env = os.environ | {"PGHOST": address.hostname or "localhost", "PGPORT": str(address.port or 5432), "PGUSER": urllib.parse.unquote(address.username or ""), "PGPASSWORD": urllib.parse.unquote(address.password or ""), "PGDATABASE": urllib.parse.unquote(address.path.lstrip("/"))}
    return subprocess.check_output(["psql", "-XAt", "-v", "ON_ERROR_STOP=1", "-c", query], text=True, env=env).strip()


def preview():
    for module in ("receivables", "payables"):
        response = call("plan", {"operations": [source(module)]})
        if not response.get("valid"):
            raise RuntimeError(f"{module}: MCP preview refused")
        facts = response["results"][0]["preview"]["result"]["changes"]
        assert len(facts["rows"]) == len(facts["reconciliation"]) == 2
        assert all(Decimal(row["difference"]) == 0 for row in facts["reconciliation"])
        assert facts["historical_control_account_codes"] == source(module)["input"]["historical_control_account_codes"]
        (evidence / (module + "-mcp-review.json")).write_text(json.dumps(facts))
        print(f"{module}: MCP preview contains both reconciled controls")


def verify():
    for module in ("receivables", "payables"):
        reply = call("read", {"key": f"finance.{module}.opening.context", "input": {"company_id": fixture["company_id"]}})
        context = reply["result"]
        facts = json.loads((evidence / (module + "-mcp-review.json")).read_text())
        assert context["batch"]["source_reference"] == source(module)["input"]["source_reference"]
        assert context["decisions"][-1]["review"]["reconciliation"] == facts["reconciliation"]
        assert context["decisions"][-1]["review"]["historical_control_account_codes"] == facts["historical_control_account_codes"]
        saved = {row["source_id"]: row["source_facts"] for row in context["items"]}
        for row in facts["rows"]:
            for field in ("account_id", "account_code", "document_reference", "document_date", "due_date", "original_total", "outstanding"):
                assert saved[row["source_id"]][field] == row[field], (module, field)
        print(f"{module}: browser approval retained the MCP-reviewed rows and reconciliation")
    company = int(fixture["company_id"])
    rows = sql(f"SELECT a.code,r.balance::text FROM finance_ledger_opening_recognitions r JOIN finance_ledger_accounts a ON a.id=r.account_id AND a.company_id=r.company_id WHERE r.tenant_id=1 AND r.company_id={company} ORDER BY a.code")
    recognized = {code: Decimal(amount) for code, amount in (line.split("|") for line in rows.splitlines())}
    assert recognized == {"1100": Decimal("2500"), "1110": Decimal("500"), "2000": Decimal("-1200"), "2010": Decimal("-300")}
    assert sql(f"SELECT count(*) FROM finance_ledger_entries WHERE tenant_id=1 AND company_id={company}") == "1"
    print("Four exact opening recognitions retained without another ledger entry")


if __name__ == "__main__":
    {"preview": preview, "verify": verify}[sys.argv[1]]()
