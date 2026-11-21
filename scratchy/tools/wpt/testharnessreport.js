// scratchy's own testharnessreport.js (QWPT4a, R64 2.2): the vendor hook testharness.js calls.
// wire serves it in place of WPT's file: `--mount-override /resources/testharnessreport.js=THIS`.
// It turns the HTML results table off and hands the results, as one JSON string, to the native
// `__scratchy_wpt_done`, which only `scratchy --wpt-th-one` installs (elsewhere this does nothing).
// Format (scratchy/NOTES.md "WPT testharness child"):
//   {"harness": "OK"|"ERROR"|"TIMEOUT"|"PRECONDITION_FAILED", "message": string|null,
//    "tests": [[name, "PASS"|"FAIL"|"TIMEOUT"|"NOTRUN"|"PRECONDITION_FAILED", message|null], ...]}
(function () {
  var HARNESS = ['OK', 'ERROR', 'TIMEOUT', 'PRECONDITION_FAILED'];
  var TEST = ['PASS', 'FAIL', 'TIMEOUT', 'NOTRUN', 'PRECONDITION_FAILED'];
  var word = function (list, n) { return list[n] || 'UNKNOWN(' + n + ')'; };
  var text = function (m) { return m === null || m === undefined ? null : String(m); };
  setup({ output: false });
  add_completion_callback(function (tests, status) {
    if (typeof __scratchy_wpt_done !== 'function') return;
    var out = { harness: word(HARNESS, status.status), message: text(status.message), tests: [] };
    for (var i = 0; i < tests.length; i++) {
      out.tests.push([String(tests[i].name), word(TEST, tests[i].status), text(tests[i].message)]);
    }
    __scratchy_wpt_done(JSON.stringify(out));
  });
})();
