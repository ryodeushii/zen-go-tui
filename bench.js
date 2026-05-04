const fs = require('fs');
const { spawn, execSync } = require('child_process');
const cwd = process.cwd();
const bin = `${cwd}/target/x86_64-unknown-linux-gnu/release/zen-go-tui`;
function sleep(ms) { return new Promise((resolve) => setTimeout(resolve, ms)); }

// ── High-precision CPU via /proc/<pid>/schedstat (nanosecond) ────────
const CLK_TCK = Number(execSync('getconf CLK_TCK').toString().trim());

function readCpuNsTree(rootPid) {
  const descendants = new Set([rootPid]);
  const queue = [rootPid];
  while (queue.length) {
    const pid = queue.shift();
    try {
      const children = execSync(`cat /proc/${pid}/task/${pid}/children 2>/dev/null || true`, { encoding: 'utf8' }).trim();
      for (const c of children.split(/\s+/)) {
        const cp = Number(c);
        if (cp && !descendants.has(cp)) { descendants.add(cp); queue.push(cp); }
      }
    } catch { /* gone */ }
  }
  let totalNs = 0;
  let pidCount = 0;
  for (const pid of descendants) {
    try {
      const sched = fs.readFileSync(`/proc/${pid}/schedstat`, 'utf8').trim();
      totalNs += Number(sched.split(/\s+/)[0]);
      pidCount++;
    } catch {
      try {
        const stat = fs.readFileSync(`/proc/${pid}/stat`, 'utf8');
        const parts = stat.replace(/^[^(]*\)/, '').trim().split(/\s+/);
        totalNs += (Number(parts[12]) + Number(parts[13])) * (1e9 / CLK_TCK);
        pidCount++;
      } catch { /* gone */ }
    }
  }
  return { cpuNs: totalNs, pidCount };
}

async function measureHighPrec(label, command, args, opts = {}) {
  const settleMs = opts.settleMs ?? 7000;
  const durationSec = opts.durationSec ?? 12;
  const sampleIntervalMs = opts.sampleIntervalMs ?? 100;

  let stdout = '';
  let stderr = '';
  const child = spawn(command, args, { cwd, detached: true, stdio: ['ignore', 'pipe', 'pipe'] });
  child.stdout.on('data', d => { stdout += d.toString(); });
  child.stderr.on('data', d => { stderr += d.toString(); });
  let exited = null;
  child.on('exit', (code, signal) => { exited = { code, signal }; });

  await sleep(settleMs);
  if (exited) return { label, started: false, exited, stdout: stdout.trim().split('\n').slice(-10), stderr: stderr.trim().split('\n').slice(-10) };

  const rootPid = child.pid;
  const samples = [];
  const numSamples = Math.round((durationSec * 1000) / sampleIntervalMs);
  let prevCpuNs = readCpuNsTree(rootPid).cpuNs;
  let prevWallNs = Number(process.hrtime.bigint());

  for (let i = 0; i < numSamples; i++) {
    await sleep(sampleIntervalMs);
    const { cpuNs: curCpuNs } = readCpuNsTree(rootPid);
    const curWallNs = Number(process.hrtime.bigint());
    const deltaCpuNs = curCpuNs - prevCpuNs;
    const deltaWallNs = curWallNs - prevWallNs;
    samples.push(deltaWallNs > 0 ? Number(((deltaCpuNs / deltaWallNs) * 100).toFixed(6)) : 0);
    prevCpuNs = curCpuNs;
    prevWallNs = curWallNs;
  }

  try { process.kill(-child.pid, 'SIGTERM'); } catch {}
  await sleep(500);
  try { process.kill(-child.pid, 'SIGKILL'); } catch {}

  const avg = samples.reduce((a, b) => a + b, 0) / samples.length;
  const min = Math.min(...samples);
  const max = Math.max(...samples);
  const stddev = Math.sqrt(samples.reduce((s, v) => s + (v - avg) ** 2, 0) / samples.length);
  return {
    label, started: true, settleSeconds: settleMs / 1000, sampleSeconds: durationSec,
    sampleIntervalMs, numSamples: samples.length,
    avgCpuPercent: Number(avg.toFixed(6)), minCpuPercent: Number(min.toFixed(6)),
    maxCpuPercent: Number(max.toFixed(6)), stddevCpuPercent: Number(stddev.toFixed(6)),
    samples, stdout: stdout.trim().split('\n').slice(-10), stderr: stderr.trim().split('\n').slice(-10),
  };
}
// ── End high-precision ───────────────────────────────────────────────

(async () => {
  const args = process.argv.slice(2);
  const headedOnly = args.includes('--headed-only');
  const headlessOnly = args.includes('--headless-only');
  const durationFlag = args.find(a => a.startsWith('--duration='));
  const durationSec = durationFlag ? Number(durationFlag.split('=')[1]) : undefined;
  console.error(`Usage: node bench.js [--headed-only] [--headless-only] [--duration=SECS] (default: 12s)`);

  try {
    if (!fs.existsSync(bin)) throw new Error(`Missing binary: ${bin}`);

    const opts = {};
    if (durationSec) opts.durationSec = durationSec;

    let result;
    if (headedOnly && headlessOnly) {
      const headless = await measureHighPrec('headless', bin, ['--headless'], opts);
      const headed = await measureHighPrec('headed', 'script', ['-q', '-c', bin, '/dev/null'], opts);
      result = { headless, headed };
    } else if (headedOnly) {
      const headed = await measureHighPrec('headed', 'script', ['-q', '-c', bin, '/dev/null'], opts);
      result = { headed };
    } else if (headlessOnly) {
      const headless = await measureHighPrec('headless', bin, ['--headless'], opts);
      result = { headless };
    } else {
      const headless = await measureHighPrec('headless', bin, ['--headless'], opts);
      const headed = await measureHighPrec('headed', 'script', ['-q', '-c', bin, '/dev/null'], opts);
      result = { headless, headed };
    }

    console.log(JSON.stringify({
      method: `/proc/<pid>/schedstat ns precision, 100ms intervals, ${durationSec ?? 12}s window`,
      clkTck: CLK_TCK,
      ...result,
    }, null, 2));
  } catch (error) {
    console.log(JSON.stringify({ error: String(error && error.stack || error) }, null, 2));
  }
})()
