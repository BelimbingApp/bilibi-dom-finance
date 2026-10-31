// Run with chrome-devtools-axi run after signing in to the disposable launcher.
// Prepend `const proofConfig = <Company 008 from BILIBI_OPENING_PROOF_CONFIG>;`.
// This uses only visible page controls and the shared browser operation doors.
if (typeof proofConfig === "undefined" || proofConfig.company_id !== 8) {
  throw new Error("Supply the disposable Company 008 credit fixture configuration");
}
const origin = await page.eval(() => location.origin);
if (!/^https:\/\/[^/]+\.localhost$/.test(origin)) {
  throw new Error("Use the task's disposable HTTPS localhost launcher");
}
const exception = "Disposable browser parity proof: the company explicitly permits and records same-person approval exceptions.";
function requireProof(condition, message) {
  if (!condition) throw new Error(message);
}
async function fillCredit(name, value) {
  await page.fill("#credit-" + name, value);
  await page.wait(350); // Let the shared Form retain its changed draft.
}
async function choose(selector, value) {
  await page.fill(selector, value);
  await page.press("ArrowDown");
  await page.press("Enter");
  await page.wait(350);
}
async function reviewAndCommit(posts) {
  await page.click("#credit-submit");
  await page.wait(450);
  const review = await page.eval(() => document.querySelector("#credit-form").innerText);
  requireProof(review.includes(posts ? "Owned ledger entry" : "No ledger entry"), "Unexpected settlement posting review");
  requireProof(review.includes("Opening credit proof counterparty"), "Review omitted the party");
  requireProof(review.includes("Account"), "Review omitted the saved controls");
  await page.click("#credit-submit");
  await page.wait(500);
  requireProof(await page.eval(() => document.querySelector("main").innerText.includes("Credit settlement recorded.")), "Browser settlement commit refused");
}
async function reverse(date, reference, posts) {
  await page.click('button[id^="credit-reverse-"]');
  await page.wait("#credit-effective_date");
  await fillCredit("effective_date", date);
  await fillCredit("reference", reference);
  await fillCredit("override_reason", exception);
  await reviewAndCommit(posts);
}
async function opening(module, source) {
  const list = module === "receivables" ? "invoices" : "bills";
  await page.open(origin + "/finance/" + module + "/" + list);
  requireProof(await page.eval(() => document.querySelector('button[aria-label^="Switch desk"]')?.textContent.includes("Company 008")), "Choose the Company 008 desk first");
  await page.click("#" + module + "-opening");
  await page.wait("#conversion-form");
  if (await page.eval(() => !!document.querySelector("#conversion-recorded"))) {
    await page.click('button[data-event="close_opening"]');
    await page.wait(300);
  } else {
    await page.fill("#conversion-source_reference", "BROWSER-CREDIT-" + source.toUpperCase());
    await page.fill("#conversion-csv", proofConfig[source]);
    const fields = await page.eval(() => [...document.querySelectorAll('#conversion-form input[name^="conversion[opening_account_classifications]"]')].map(e => ({ account: e.name.match(/\[(\d+)\]/)[1], selector: "#" + e.id.replace(/-value$/, "") })));
    for (const field of fields) {
      const kind = proofConfig.opening_account_classifications[field.account];
      if (kind) await choose(field.selector, { not_a_control: "Not a control", receivables_control: "Receivables control", payables_control: "Payables control" }[kind]);
    }
    if (module === "receivables") await choose("#conversion-series_id", "Opening");
    await page.click("#conversion-preview");
    await page.wait("#conversion-import");
    const review = await page.eval(() => document.querySelector("main").innerText);
    requireProof(review.includes("5 records") && review.includes('"status":"unknown"') && review.includes('"tax_total":"0"'), "Opening review omitted source rows or tax distinctions");
    await page.click("#conversion-import");
    await page.wait(600);
  }
  const records = await page.eval(() => [...document.querySelectorAll("main a[href]")].map(a => a.getAttribute("href")).filter(href => /\/(?:invoices|bills)\/\d+$/.test(href)));
  requireProof(records.length === 5, "Expected the fixture's five mixed opening records on their existing list");
  return { primary: records[3], historical: records[2] };
}
for (const [module, source] of [["receivables", "ar"], ["payables", "ap"]]) {
  const records = await opening(module, source);
  const target = module === "receivables" ? "OPEN-0001" : "ORIGINAL-INVOICE";
  await page.open(origin + records.primary);
  const facts = await page.eval(() => document.querySelector("main").innerText);
  requireProof(facts.includes("53.00") && facts.includes('"tax_total":"3"'), "Record lost its source gross or tax evidence");
  await page.click("#credit-apply");
  await page.wait("#credit-effective_date");
  await fillCredit("effective_date", "2026-07-16");
  await fillCredit("amount", "31");
  await fillCredit("reference", "BROWSER-proof-apply");
  await choose("#credit-target_document_id", target);
  await fillCredit("override_reason", exception);
  await page.click("#credit-submit");
  await page.wait(450);
  requireProof(await page.eval(() => document.querySelector("#credit-amount")?.getAttribute("aria-invalid") === "true"), "Over-application refusal did not stay beside Amount");
  await fillCredit("amount", "20");
  await reviewAndCommit(false);
  await reverse("2026-07-17", "BROWSER-proof-reverse-apply", false);
  await page.click("#credit-refund");
  await page.wait("#credit-effective_date");
  await fillCredit("effective_date", "2026-07-22");
  await fillCredit("amount", "5");
  await fillCredit("reference", "BROWSER-proof-refund");
  await choose("#credit-cash_account_id", "Cash at bank");
  await fillCredit("override_reason", exception);
  await reviewAndCommit(true);
  await reverse("2026-07-26", "BROWSER-proof-reverse-refund", true);
  await page.open(origin + records.historical);
  await page.click("#credit-apply");
  await page.wait("#credit-effective_date");
  await fillCredit("effective_date", "2026-07-20");
  await fillCredit("amount", "10");
  await fillCredit("reference", "BROWSER-proof-transfer");
  await choose("#credit-target_document_id", target);
  await fillCredit("override_reason", exception);
  await reviewAndCommit(true);
  await reverse("2026-07-27", "BROWSER-proof-reverse-transfer", true);
  console.log("PASS browser " + module + ": mixed import/source evidence, Amount refusal, entryless application/reversal, cash refund/reversal and saved-control transfer/reversal through shared reviewed contracts");
}
