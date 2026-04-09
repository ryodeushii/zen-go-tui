const fs = require('fs');
const { spawn, execSync } = require('child_process');
const cwd = '/home/ryodeushii/repos/antelope-analysis-gpt54';
const bin = `${cwd}/target/x86_64-unknown-linux-gnu/release/zen-go-tui`;
function sleep(ms) { return new Promise((resolve) => setTimeout(resolve, ms)); }
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
  try {
    if (!fs.existsSync(bin)) throw new Error(`Missing binary: ${bin}`);
    const headless = await measure('headless', bin, ['--headless']);
    const headed = await measure('headed', 'script', ['-q', '-c', bin, '/dev/null']);
    console.log(JSON.stringify({ method: 'built release binary, 7s settle, then 12 x 1s ps tree %CPU samples averaged', headless, headed }, null, 2));
  } catch (error) {
    console.log(JSON.stringify({ error: String(error && error.stack || error) }, null, 2));
  }
})()
