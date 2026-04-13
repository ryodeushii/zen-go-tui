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

function getPsTable() {
  const out = execSync('ps -e -o pid=,ppid=,%cpu=,comm=', { encoding: 'utf8' });
  return out.split('\n').map((line) => line.trim()).filter(Boolean).map((line) => {
    const m = line.match(/^(\d+)\s+(\d+)\s+([\d.]+)\s+(.+)$/);
    return m ? { pid: Number(m[1]), ppid: Number(m[2]), cpu: Number(m[3]), comm: m[4] } : null;
  }).filter(Boolean);
}
function descendantSet(rootPid, table) {
  const map = new Map();
  for (const row of table) {
    if (!map.has(row.ppid)) map.set(row.ppid, []);
    map.get(row.ppid).push(row.pid);
  }
  const seen = new Set([rootPid]);
  const queue = [rootPid];
  while (queue.length) {
    const pid = queue.shift();
    for (const child of map.get(pid) || []) {
      if (!seen.has(child)) {
        seen.add(child);
        queue.push(child);
      }
    }
  }
  return seen;
}
function sampleTree(rootPid) {
  const table = getPsTable();
  const pids = descendantSet(rootPid, table);
  const members = table.filter((row) => pids.has(row.pid));
  return {
    cpu: members.reduce((sum, row) => sum + row.cpu, 0),
    members: members.map(({ pid, comm, cpu }) => ({ pid, comm, cpu })),
  };
}
async function measure(label, command, args) {
  let stdout = '';
  let stderr = '';
  const child = spawn(command, args, { cwd, detached: true, stdio: ['ignore', 'pipe', 'pipe'] });
  child.stdout.on('data', (d) => { stdout += d.toString(); });
  child.stderr.on('data', (d) => { stderr += d.toString(); });
  let exited = null;
  child.on('exit', (code, signal) => { exited = { code, signal }; });
  await sleep(7000);
  if (exited) {
    return { label, started: false, exited, stdout: stdout.trim().split('\n').slice(-10), stderr: stderr.trim().split('\n').slice(-10) };
  }
  const samples = [];
  let members = [];
  for (let i = 0; i < 12; i++) {
    const sample = sampleTree(child.pid);
    samples.push(sample.cpu);
    members = sample.members;
    await sleep(1000);
  }
  try { process.kill(-child.pid, 'SIGTERM'); } catch {}
  await sleep(500);
  try { process.kill(-child.pid, 'SIGKILL'); } catch {}
  const avg = samples.reduce((a, b) => a + b, 0) / samples.length;
  return {
    label,
    started: true,
    settleSeconds: 7,
    sampleSeconds: 12,
    samples,
    avgCpuPercent: Number(avg.toFixed(3)),
    members,
    stdout: stdout.trim().split('\n').slice(-10),
    stderr: stderr.trim().split('\n').slice(-10),
  };
}
(async () => {
  const args = process.argv.slice(2);
  const modeFlag = args.find(a => a.startsWith('--mode='));
  const mode = modeFlag ? modeFlag.split('=')[1] : 'schedstat';
  const headedOnly = args.includes('--headed-only');
  const headlessOnly = args.includes('--headless-only');
  const durationFlag = args.find(a => a.startsWith('--duration='));
  const durationSec = durationFlag ? Number(durationFlag.split('=')[1]) : undefined;
  if (!['ps', 'schedstat'].includes(mode)) {
    console.error(`Usage: node bench.js [--mode=ps|schedstat] [--headed-only] [--headless-only] [--duration=SECS] (default: schedstat, both modes, 12s)`);
    process.exit(1);
  }

  try {
    if (!fs.existsSync(bin)) throw new Error(`Missing binary: ${bin}`);

    const runPs = async () => {
      if (headedOnly && headlessOnly) {
        const headless = await measure('headless', bin, ['--headless']);
        const headed = await measure('headed', 'script', ['-q', '-c', bin, '/dev/null']);
        return { headless, headed };
      }
      if (headedOnly) {
        const headed = await measure('headed', 'script', ['-q', '-c', bin, '/dev/null']);
        return { headed };
      }
      if (headlessOnly) {
        const headless = await measure('headless', bin, ['--headless']);
        return { headless };
      }
      const headless = await measure('headless', bin, ['--headless']);
      const headed = await measure('headed', 'script', ['-q', '-c', bin, '/dev/null']);
      return { headless, headed };
    };

    const runSchedstat = async () => {
      const opts = {};
      if (durationSec) opts.durationSec = durationSec;
      if (headedOnly && headlessOnly) {
        const headless = await measureHighPrec('headless', bin, ['--headless'], opts);
        const headed = await measureHighPrec('headed', 'script', ['-q', '-c', bin, '/dev/null'], opts);
        return { headless, headed };
      }
      if (headedOnly) {
        const headed = await measureHighPrec('headed', 'script', ['-q', '-c', bin, '/dev/null'], opts);
        return { headed };
      }
      if (headlessOnly) {
        const headless = await measureHighPrec('headless', bin, ['--headless'], opts);
        return { headless };
      }
      const headless = await measureHighPrec('headless', bin, ['--headless'], opts);
      const headed = await measureHighPrec('headed', 'script', ['-q', '-c', bin, '/dev/null'], opts);
      return { headless, headed };
    };

    if (mode === 'ps') {
      const result = await runPs();
      console.log(JSON.stringify({
        method: 'ps %CPU, 7s settle, 12 x 1s tree samples averaged',
        ...result,
      }, null, 2));
    } else {
      const result = await runSchedstat();
      console.log(JSON.stringify({
        method: `/proc/<pid>/schedstat ns precision, 100ms intervals, ${durationSec ?? 12}s window`,
        clkTck: CLK_TCK,
        ...result,
      }, null, 2));
    }
  } catch (error) {
    console.log(JSON.stringify({ error: String(error && error.stack || error) }, null, 2));
  }
})()
