use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{SurfnetError, SurfnetResult};

/// Transaction data from a single Surfnet instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurfnetReportData {
    pub instance_id: String,
    pub test_name: Option<String>,
    pub rpc_url: String,
    pub transactions: Vec<TransactionReportEntry>,
    pub timestamp: String,
}

/// A single transaction with its full profile data.
///
/// Profiles are stored as raw JSON values in both `jsonParsed` and `base64`
/// encodings — the same format the studio UI consumes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionReportEntry {
    pub signature: String,
    pub slot: u64,
    pub error: Option<String>,
    pub logs: Vec<String>,
    pub profile_json_parsed: Option<serde_json::Value>,
    pub profile_base64: Option<serde_json::Value>,
}

/// A consolidated test report covering all Surfnet instances from a test suite.
///
/// ```rust,no_run
/// use surfpool_sdk::report::SurfpoolReport;
///
/// // After tests complete:
/// let report = SurfpoolReport::from_directory("target/surfpool-reports").unwrap();
/// report.write_html("target/surfpool-report.html").unwrap();
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurfpoolReport {
    pub instances: Vec<SurfnetReportData>,
    pub generated_at: String,
}

impl SurfpoolReport {
    /// Read and consolidate all report JSON files from a directory.
    ///
    /// Each file was written by a `Surfnet` instance via `write_report_data()`.
    pub fn from_directory(dir: impl AsRef<Path>) -> SurfnetResult<Self> {
        let dir = dir.as_ref();

        if !dir.exists() {
            return Err(SurfnetError::Report(format!(
                "report directory does not exist: {}",
                dir.display()
            )));
        }

        let mut instances = Vec::new();

        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .map_err(|e| SurfnetError::Report(format!("failed to read report dir: {e}")))?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
            .collect();

        // Sort by filename for deterministic ordering
        entries.sort_by_key(|e| e.file_name());

        for entry in entries {
            let path = entry.path();
            let content = std::fs::read_to_string(&path).map_err(|e| {
                SurfnetError::Report(format!("failed to read {}: {e}", path.display()))
            })?;

            let data: SurfnetReportData = serde_json::from_str(&content).map_err(|e| {
                SurfnetError::Report(format!("failed to parse {}: {e}", path.display()))
            })?;

            instances.push(data);
        }

        Ok(SurfpoolReport {
            instances,
            generated_at: chrono::Utc::now().to_rfc3339(),
        })
    }

    /// Total number of transactions across all instances.
    pub fn total_transactions(&self) -> usize {
        self.instances.iter().map(|i| i.transactions.len()).sum()
    }

    /// Number of failed transactions across all instances.
    pub fn failed_transactions(&self) -> usize {
        self.instances
            .iter()
            .flat_map(|i| &i.transactions)
            .filter(|t| t.error.is_some())
            .count()
    }

    /// Generate the HTML report as a string.
    ///
    /// This produces a self-contained HTML page with all transaction data
    /// embedded as JSON, rendered by the Surfpool report UI.
    pub fn generate_html(&self) -> SurfnetResult<String> {
        let json = serde_json::to_string(self)
            .map_err(|e| SurfnetError::Report(format!("failed to serialize report: {e}")))?;

        // For now, generate a standalone HTML report with embedded data.
        // Once the studio report UI template is built and embedded via build.rs,
        // this will use: REPORT_TEMPLATE.replace("__REPORT_DATA_PLACEHOLDER__", &json)
        Ok(generate_standalone_html(&json))
    }

    /// Generate the HTML report and write it to a file.
    pub fn write_html(&self, path: impl AsRef<Path>) -> SurfnetResult<()> {
        let html = self.generate_html()?;
        let path = path.as_ref();

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| SurfnetError::Report(format!("failed to create output dir: {e}")))?;
        }

        std::fs::write(path, html)
            .map_err(|e| SurfnetError::Report(format!("failed to write report: {e}")))?;

        log::info!("Surfpool report written to {}", path.display());
        Ok(())
    }
}

/// Generate a standalone HTML report styled to match surfpool.run.
fn generate_standalone_html(report_json: &str) -> String {
    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Surfpool Test Report</title>
<link rel="preconnect" href="https://fonts.googleapis.com"><link rel="preconnect" href="https://fonts.gstatic.com" crossorigin><link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&display=swap" rel="stylesheet">
<script id="__SURFPOOL_REPORT_DATA__" type="application/json">{report_json}</script>
<style>
:root {{--bg:#000;--s1:#09090b;--s2:#18181b;--s3:#27272a;--b:#27272a;--b2:#3f3f46;--t:#e4e4e7;--t2:#a1a1aa;--t3:#71717a;--cyan:#00D4FF;--cyan-dim:rgba(0,212,255,.1);--cyan-border:#0a2f3d;--cyan-bg:#0a2233;--g:#10b981;--r:#ef4444;--y:#eab308;--m:'SF Mono',ui-monospace,'Cascadia Code','Fira Code',monospace;--f:Inter,-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,sans-serif}}
*{{margin:0;padding:0;box-sizing:border-box}}
body{{font-family:var(--f);background:var(--bg);color:var(--t);-webkit-font-smoothing:antialiased;-moz-osx-font-smoothing:grayscale;padding:0;line-height:1.5}}
.wrap{{max-width:1280px;margin:0 auto;padding:32px 24px}}

/* Header with subtle cyan glow */
.hdr{{padding:48px 0 32px;position:relative}}
.hdr::before{{content:'';position:absolute;top:-80px;left:50%;transform:translateX(-50%);width:600px;height:300px;background:var(--cyan);opacity:.04;border-radius:50%;filter:blur(80px);pointer-events:none}}
.hdr h1{{font-size:20px;font-weight:700;letter-spacing:-.5px;display:flex;align-items:center;gap:10px}}
.hdr h1 .accent{{color:var(--cyan);font-family:var(--m)}}
.hdr .sub{{color:var(--t3);font-size:12px;margin-top:6px;font-family:var(--m)}}

/* Stats grid */
.stats{{display:grid;grid-template-columns:repeat(4,1fr);gap:12px;margin-bottom:32px}}
.st{{background:var(--s1);border:1px solid var(--b);border-radius:8px;padding:16px}}
.st .l{{font-size:10px;color:var(--t3);text-transform:uppercase;letter-spacing:1px;font-weight:500;font-family:var(--m)}}
.st .v{{font-size:24px;font-weight:700;margin-top:4px;font-family:var(--m);letter-spacing:-.5px}}
.st .v.g{{color:var(--g)}}.st .v.r{{color:var(--r)}}.st .v.c{{color:var(--cyan)}}

/* Test instance cards */
.inst{{background:var(--s1);border:1px solid var(--b);border-radius:12px;margin-bottom:12px;overflow:hidden}}
.inst-h{{padding:12px 16px;display:flex;justify-content:space-between;align-items:center;cursor:pointer;user-select:none;transition:background .15s}}
.inst-h:hover{{background:var(--s2)}}
.inst-h h2{{font-size:13px;font-weight:600;font-family:var(--m);color:var(--t)}}
.inst-h .tags{{display:flex;gap:6px;align-items:center}}
.tag{{font-size:9px;padding:3px 8px;border-radius:9999px;font-weight:500;font-family:var(--m);text-transform:uppercase;letter-spacing:.5px}}
.tag.cnt{{background:var(--s2);color:var(--t2);border:1px solid var(--b)}}
.tag.fail{{background:rgba(239,68,68,.1);color:var(--r);border:1px solid rgba(239,68,68,.2)}}
.tag.cu{{background:var(--cyan-dim);color:var(--cyan);border:1px solid var(--cyan-border)}}
.tag.pass{{background:rgba(16,185,129,.1);color:var(--g);border:1px solid rgba(16,185,129,.2)}}
.inst-body{{display:none;border-top:1px solid var(--b)}}
.inst-body.open{{display:block}}

/* Transaction table */
table{{width:100%;border-collapse:collapse;font-size:12px}}
th{{text-align:left;padding:8px 14px;color:var(--t3);font-size:9px;text-transform:uppercase;letter-spacing:.8px;font-weight:500;font-family:var(--m);border-bottom:1px solid var(--b);background:var(--s2)}}
td{{padding:8px 14px;border-bottom:1px solid rgba(39,39,42,.6);vertical-align:middle}}
tr:last-child td{{border-bottom:none}}
tr:hover td{{background:rgba(24,24,27,.8)}}
tr.clickable{{cursor:pointer}}

.dot{{width:7px;height:7px;border-radius:50%;display:inline-block;margin-right:8px;vertical-align:middle;flex-shrink:0}}
.dot.ok{{background:var(--g)}}.dot.err{{background:var(--r)}}
.sig-cell{{display:flex;align-items:center}}
.sig{{font-family:var(--m);color:var(--cyan);font-size:11px}}
.cuv{{font-family:var(--m);color:var(--cyan);font-size:11px}}
.programs{{display:flex;gap:4px;flex-wrap:wrap}}
.pgm{{font-size:9px;padding:2px 6px;border-radius:4px;background:var(--s3);color:var(--t2);font-family:var(--m);white-space:nowrap;border:1px solid var(--b2)}}
.log-preview{{color:var(--t3);font-size:10px;font-family:var(--m);max-width:500px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}}
.log-preview .hi{{color:var(--g)}}
.log-preview .er{{color:var(--r)}}
.cu-mini{{display:flex;gap:1px;height:5px;border-radius:3px;overflow:hidden;min-width:60px;max-width:120px;margin-top:4px}}
.cu-mini .seg{{height:100%;min-width:2px;border-radius:1px}}
.empty{{padding:24px;text-align:center;color:var(--t3);font-size:12px;font-family:var(--m)}}

/* Modal */
.modal-bg{{display:none;position:fixed;inset:0;background:rgba(0,0,0,.8);backdrop-filter:blur(4px);-webkit-backdrop-filter:blur(4px);z-index:100;align-items:start;justify-content:center;padding:40px;overflow-y:auto}}
.modal-bg.open{{display:flex}}
.modal{{background:var(--s1);border:1px solid var(--b);border-radius:12px;max-width:900px;width:100%;max-height:90vh;overflow-y:auto}}
.modal .mh{{padding:14px 18px;border-bottom:1px solid var(--b);display:flex;justify-content:space-between;align-items:center;position:sticky;top:0;background:var(--s1);z-index:1}}
.modal .mh h3{{font-size:12px;font-family:var(--m);color:var(--cyan)}}
.modal .mc{{background:none;border:none;color:var(--t3);cursor:pointer;font-size:20px;line-height:1;transition:color .15s}}.modal .mc:hover{{color:var(--t)}}
.modal .mb{{padding:18px}}
.dr{{display:flex;gap:8px;margin-bottom:5px;font-size:12px}}.dr .k{{color:var(--t3);min-width:110px;flex-shrink:0;font-family:var(--m);font-size:11px}}
.sec{{margin-bottom:18px}}.sec h4{{font-size:9px;color:var(--t3);text-transform:uppercase;letter-spacing:1px;margin-bottom:8px;font-weight:500;font-family:var(--m)}}

/* Logs */
.logs{{background:var(--bg);border:1px solid var(--b);border-radius:8px;padding:12px 14px;font-size:10px;font-family:var(--m);line-height:1.8;max-height:350px;overflow-y:auto;white-space:pre-wrap;word-break:break-all}}
.ll{{color:var(--t3)}}.ll.p{{color:var(--g)}}.ll.e{{color:var(--r)}}.ll.d{{color:var(--y)}}

/* CU bar */
.cu-bar{{display:flex;gap:2px;height:16px;border-radius:4px;overflow:hidden;margin:8px 0;background:var(--s2)}}
.cu-bar .seg{{height:100%;min-width:2px}}

/* Instructions */
.ix{{border:1px solid var(--b);border-radius:8px;margin-bottom:6px;overflow:hidden}}
.ix-h{{padding:8px 14px;background:var(--s2);font-size:11px;display:flex;justify-content:space-between;cursor:pointer;font-family:var(--m);transition:background .15s}}.ix-h:hover{{background:var(--s3)}}
.ix-b{{padding:12px 14px;display:none}}.ix-b.open{{display:block}}

/* Account changes */
.ac{{background:var(--bg);border:1px solid var(--b);border-radius:8px;margin-top:8px;overflow:hidden}}
.ac-h{{padding:10px 12px;display:flex;align-items:center;gap:8px;cursor:pointer;transition:background .1s}}
.ac-h:hover{{background:var(--s2)}}
.ac .addr{{color:var(--cyan);font-family:var(--m);font-size:11px}}.ac .ct{{font-size:8px;padding:2px 6px;border-radius:9999px;display:inline-block;text-transform:uppercase;letter-spacing:.5px;font-family:var(--m);font-weight:500}}
.ac .ct.create{{background:rgba(16,185,129,.1);color:var(--g);border:1px solid rgba(16,185,129,.2)}}
.ac .ct.update{{background:rgba(234,179,8,.1);color:var(--y);border:1px solid rgba(234,179,8,.2)}}
.ac .ct.delete{{background:rgba(239,68,68,.1);color:var(--r);border:1px solid rgba(239,68,68,.2)}}
.ac .ct.unchanged{{background:var(--s2);color:var(--t3);border:1px solid var(--b)}}
.ac .ct.read{{background:var(--s2);color:var(--t3);border:1px solid var(--b)}}
.ac-b{{display:none;border-top:1px solid var(--b);padding:10px 12px}}.ac-b.open{{display:block}}
.ac-grid{{display:grid;grid-template-columns:auto 1fr;gap:4px 12px;font-size:11px;font-family:var(--m)}}
.ac-grid .ak{{color:var(--t3);white-space:nowrap}}.ac-grid .av{{color:var(--t)}}
.ac-grid .av.changed{{color:var(--y)}}
.diff-row{{display:flex;align-items:center;gap:6px}}
.diff-row .before{{color:var(--t3)}}.diff-row .arrow{{color:var(--t3);font-size:10px}}.diff-row .after{{color:var(--t)}}
.diff-row .after.up{{color:var(--g)}}.diff-row .after.down{{color:var(--r)}}
.hex-block{{background:var(--s1);border:1px solid var(--b);border-radius:6px;padding:8px 10px;font-family:var(--m);font-size:10px;line-height:1.6;max-height:200px;overflow:auto;white-space:pre;color:var(--t3);margin-top:4px}}
.hex-block .add{{color:var(--g);background:rgba(16,185,129,.08)}}.hex-block .rem{{color:var(--r);background:rgba(239,68,68,.08)}}.hex-block .chg{{color:var(--y);background:rgba(234,179,8,.08)}}
.json-block{{background:var(--s1);border:1px solid var(--b);border-radius:6px;padding:8px 10px;font-family:var(--m);font-size:10px;line-height:1.5;max-height:250px;overflow:auto;white-space:pre-wrap;word-break:break-all;color:var(--t2);margin-top:4px}}
.json-block .jk{{color:var(--cyan)}}.json-block .js{{color:var(--g)}}.json-block .jn{{color:var(--y)}}
.ro-section{{margin-top:12px;padding:10px 12px;background:var(--bg);border:1px solid var(--b);border-radius:8px}}
.ro-section .ro-addr{{font-family:var(--m);font-size:10px;color:var(--t3);padding:3px 0;display:flex;justify-content:space-between}}
</style>
</head>
<body>
<div class="wrap" id="app"></div>
<script>
(function(){{
const raw=document.getElementById('__SURFPOOL_REPORT_DATA__');
if(!raw)return;
const R=JSON.parse(raw.textContent), app=document.getElementById('app');
const COLORS=['#00D4FF','#a855f7','#ec4899','#f97316','#14b8a6','#eab308','#10b981','#6366f1'];
const PROGRAMS={{
  '11111111111111111111111111111111':'System',
  'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA':'Token',
  'ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL':'AToken',
  'ComputeBudget111111111111111111111111111111':'Budget',
  'TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb':'Token22',
}};
const esc=s=>s.replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;');
const short=s=>s.length>12?s.slice(0,6)+'..'+s.slice(-4):s;
const pgmName=a=>PROGRAMS[a]||short(a);

let totTx=0,failTx=0,totCU=0;
R.instances.forEach(i=>{{
  totTx+=i.transactions.length;
  i.transactions.forEach(t=>{{
    if(t.error)failTx++;
    totCU+=(t.profile_json_parsed?.transactionProfile?.computeUnitsConsumed||0);
  }});
}});

// Extract programs from profile
function txPrograms(tx){{
  const p=tx.profile_json_parsed;
  if(!p?.instructionProfiles)return [];
  const seen=new Set();
  p.instructionProfiles.forEach(ix=>{{
    Object.keys(ix.accountStates||{{}}).forEach(a=>{{
      // Heuristic: if an account appears as writable in multiple ix, it's likely the program
    }});
    (ix.logMessages||[]).forEach(l=>{{
      const m=l.match(/^Program (\w+) invoke/);if(m)seen.add(m[1]);
    }});
  }});
  (tx.logs||[]).forEach(l=>{{
    const m=l.match(/^Program (\w+) invoke/);if(m)seen.add(m[1]);
  }});
  return[...seen];
}}

function txCU(tx){{return tx.profile_json_parsed?.transactionProfile?.computeUnitsConsumed||0}}
function logPreview(tx){{
  const logs=tx.logs||[];
  const interesting=logs.filter(l=>l.startsWith('Program log:')||l.includes('failed')||l.includes('error'));
  if(interesting.length===0)return logs.length>0?`${{logs.length}} log lines`:'';
  const l=interesting[0];
  if(l.includes('failed')||l.includes('error'))return`<span class="er">${{esc(l.slice(0,80))}}</span>`;
  return`<span class="hi">${{esc(l.replace('Program log: ','').slice(0,80))}}</span>`;
}}

function cuMiniBar(tx){{
  const ixs=tx.profile_json_parsed?.instructionProfiles||[];
  if(ixs.length===0)return'';
  const tot=ixs.reduce((s,x)=>s+(x.computeUnitsConsumed||0),0);
  if(tot===0)return'';
  return`<div class="cu-mini">${{ixs.map((x,i)=>{{
    const pct=tot>0?(x.computeUnitsConsumed/tot*100):0;
    return`<div class="seg" style="width:${{Math.max(pct,3)}}%;background:${{COLORS[i%COLORS.length]}}" title="Ix ${{i}}: ${{(x.computeUnitsConsumed||0).toLocaleString()}} CU"></div>`;
  }}).join('')}}</div>`;
}}

app.innerHTML=`
<div class="hdr"><div><h1><span class="accent">surfpool</span> test report</h1><div class="sub">${{R.instances.length}} test${{R.instances.length!==1?'s':''}} &middot; ${{totTx}} transaction${{totTx!==1?'s':''}} &middot; ${{R.generated_at}}</div></div></div>
<div class="stats">
  <div class="st"><div class="l">Tests</div><div class="v c">${{R.instances.length}}</div></div>
  <div class="st"><div class="l">Transactions</div><div class="v">${{totTx}}</div></div>
  <div class="st"><div class="l">Failed</div><div class="v${{failTx>0?' r':''}}">${{failTx}}</div></div>
  <div class="st"><div class="l">Total CU</div><div class="v">${{totCU.toLocaleString()}}</div></div>
</div>
<div id="insts"></div>`;

const insts=document.getElementById('insts');
R.instances.forEach((inst,idx)=>{{
  const el=document.createElement('div');el.className='inst';
  const name=inst.test_name||`test_${{idx+1}}`;
  const txs=inst.transactions, tc=txs.length;
  const fc=txs.filter(t=>t.error).length;
  const icu=txs.reduce((s,t)=>s+txCU(t),0);

  let tableRows='';
  if(tc===0){{
    tableRows='<tr><td colspan="6" class="empty">No transactions</td></tr>';
  }}else{{
    txs.forEach((tx,ti)=>{{
      const cu=txCU(tx);
      const progs=txPrograms(tx);
      const lp=logPreview(tx);
      tableRows+=`<tr class="clickable" onclick="showTx(${{idx}},${{ti}})">
        <td><span class="dot ${{tx.error?'err':'ok'}}"></span><span class="sig">${{short(tx.signature)}}</span></td>
        <td style="font-family:var(--m);font-size:11px;color:var(--t2)">${{tx.slot}}</td>
        <td>${{cu>0?`<span class="cuv">${{cu.toLocaleString()}}</span>${{cuMiniBar(tx)}}`:'-'}}</td>
        <td><div class="programs">${{progs.map(p=>`<span class="pgm">${{pgmName(p)}}</span>`).join('')||'-'}}</div></td>
        <td><div class="log-preview">${{lp||'-'}}</div></td>
      </tr>`;
    }});
  }}

  el.innerHTML=`
    <div class="inst-h" onclick="this.nextElementSibling.classList.toggle('open')">
      <h2>${{esc(name)}}</h2>
      <div class="tags">
        <span class="tag cnt">${{tc}} tx${{tc!==1?'s':''}}</span>
        ${{fc>0?`<span class="tag fail">${{fc}} failed</span>`:`<span class="tag pass">pass</span>`}}
        ${{icu>0?`<span class="tag cu">${{icu.toLocaleString()}} CU</span>`:''}}
      </div>
    </div>
    <div class="inst-body${{tc>0?' open':''}}">
      <table><thead><tr><th>Signature</th><th>Slot</th><th>Compute Units</th><th>Programs</th><th>Logs</th></tr></thead>
      <tbody>${{tableRows}}</tbody></table>
    </div>`;
  insts.appendChild(el);
}});

// Modal
const mdl=document.createElement('div');mdl.className='modal-bg';
mdl.innerHTML='<div class="modal"><div class="mh"><h3 id="mt"></h3><button class="mc" onclick="closeTx()">&times;</button></div><div class="mb" id="mb"></div></div>';
document.body.appendChild(mdl);
mdl.addEventListener('click',e=>{{if(e.target===mdl)closeTx()}});
window.closeTx=()=>mdl.classList.remove('open');

// Merge jsonParsed + base64 profiles.
// jsonParsed has data.parsed (structured), base64 has data=["base64str","base64"] (raw bytes).
// We copy the raw base64 data into the jsonParsed profile as _b64 on each account snapshot.
function mergeProfiles(jp,b64){{
  if(!b64||!jp)return jp;
  const m=JSON.parse(JSON.stringify(jp));
  function mergeStates(ps,bs){{
    if(!ps||!bs)return;
    for(const[a,s]of Object.entries(bs)){{
      if(!ps[a])continue;
      const pc=ps[a].accountChange, bc=s.accountChange;
      if(!pc||!bc)continue;
      // update: data is [before, after]
      if(Array.isArray(pc.data)&&Array.isArray(bc.data)&&pc.data.length===2&&bc.data.length===2){{
        for(let i=0;i<2;i++){{
          if(bc.data[i]&&bc.data[i].data)pc.data[i]._b64=bc.data[i].data;
        }}
      }}
      // create/delete: data is a single object
      else if(pc.data&&!Array.isArray(pc.data)&&bc.data&&!Array.isArray(bc.data)){{
        if(bc.data.data)pc.data._b64=bc.data.data;
      }}
    }}
  }}
  mergeStates(m.transactionProfile?.accountStates,b64.transactionProfile?.accountStates);
  if(m.instructionProfiles&&b64.instructionProfiles){{
    for(let i=0;i<m.instructionProfiles.length&&i<b64.instructionProfiles.length;i++)
      mergeStates(m.instructionProfiles[i]?.accountStates,b64.instructionProfiles[i]?.accountStates);
  }}
  return m;
}}

function solAmount(lmp){{return(lmp/1e9).toFixed(lmp%1e9===0?0:4)+' SOL'}}
function b64toHex(b64str){{
  try{{const bin=atob(b64str);let h='';for(let i=0;i<bin.length;i++)h+=bin.charCodeAt(i).toString(16).padStart(2,'0');return h;}}
  catch{{return''}}
}}
function hexDump(hex,cols=32){{
  if(!hex)return'';
  let out='';
  for(let i=0;i<hex.length;i+=cols){{
    const chunk=hex.slice(i,i+cols);
    const offset=(i/2).toString(16).padStart(4,'0');
    const bytes=chunk.match(/.{{1,2}}/g)||[];
    const ascii=bytes.map(b=>{{const c=parseInt(b,16);return c>=32&&c<127?String.fromCharCode(c):'.';}}).join('');
    out+=`${{offset}}  ${{bytes.join(' ')}}  ${{ascii}}\n`;
  }}
  return out;
}}

// Hex dump with diff highlighting: mark bytes that differ from `other`
function hexDumpDiff(hex,other,isBefore,cols=32){{
  if(!hex)return'';
  let out='';
  for(let i=0;i<hex.length;i+=cols){{
    const chunk=hex.slice(i,i+cols);
    const otherChunk=other.slice(i,i+cols);
    const offset=(i/2).toString(16).padStart(4,'0');
    const bytes=chunk.match(/.{{1,2}}/g)||[];
    const otherBytes=(otherChunk.match(/.{{1,2}}/g)||[]);
    let hexPart='',asciiPart='';
    bytes.forEach((b,j)=>{{
      const ob=otherBytes[j]||'';
      const changed=b!==ob;
      const c=parseInt(b,16);
      const ch=c>=32&&c<127?String.fromCharCode(c):'.';
      if(changed){{
        const cls=isBefore?'rem':'add';
        hexPart+=`<span class="${{cls}}">${{b}}</span>`+(j<bytes.length-1?' ':'');
        asciiPart+=`<span class="${{cls}}">${{esc(ch)}}</span>`;
      }}else{{
        hexPart+=b+(j<bytes.length-1?' ':'');
        asciiPart+=esc(ch);
      }}
    }});
    out+=`${{offset}}  ${{hexPart}}  ${{asciiPart}}\n`;
  }}
  return out;
}}

// Extract parsed JSON from a profile account snapshot.
// In jsonParsed encoding, data can be: {{parsed:{{info:...}}}} or ['','base64']
function getParsed(snap){{
  if(!snap||!snap.data)return null;
  if(typeof snap.data==='object'&&!Array.isArray(snap.data)&&snap.data.parsed)return snap.data.parsed;
  return null;
}}
// Extract base64 raw data — from _b64 (merged) or data if it's [str,'base64']
function getB64(snap){{
  if(!snap)return'';
  if(snap._b64&&Array.isArray(snap._b64)&&snap._b64.length>=2)return snap._b64[0];
  if(Array.isArray(snap.data)&&snap.data.length>=2&&snap.data[1]==='base64'&&snap.data[0])return snap.data[0];
  return'';
}}

function renderAccountDetail(addr,state){{
  const c=state.accountChange;
  if(!c)return'';
  const ct=c.type;
  if(ct==='unchanged')return'';
  const isUpdate=ct==='update'&&Array.isArray(c.data)&&c.data.length===2;
  const isCreate=ct==='create'&&c.data&&!Array.isArray(c.data);
  const isDelete=ct==='delete'&&c.data&&!Array.isArray(c.data);
  const before=isUpdate?c.data[0]:isDelete?c.data:null;
  const after=isUpdate?c.data[1]:isCreate?c.data:null;
  const uid=Math.random().toString(36).slice(2,8);

  let h=`<div class="ac"><div class="ac-h" onclick="document.getElementById('acb_${{uid}}').classList.toggle('open')">
    <span class="ct ${{ct}}">${{ct}}</span>
    <span class="addr">${{addr}}</span>`;

  // Inline lamport diff
  if(isUpdate&&before&&after&&before.lamports!==after.lamports){{
    const diff=after.lamports-before.lamports;
    h+=`<span style="margin-left:auto;font-family:var(--m);font-size:10px"><span style="color:var(--t3)">${{solAmount(before.lamports)}}</span> <span style="color:var(--t3)">→</span> <span style="color:var(${{diff>0?'--g':'--r'}})">${{solAmount(after.lamports)}}</span> <span style="color:var(${{diff>0?'--g':'--r'}});font-size:9px">(${{diff>0?'+':''}}${{solAmount(diff)}})</span></span>`;
  }}else if(after&&(isCreate)){{
    h+=`<span style="margin-left:auto;font-family:var(--m);font-size:10px;color:var(--g)">${{solAmount(after.lamports)}}</span>`;
  }}

  h+=`</div><div class="ac-b" id="acb_${{uid}}">`;

  // Account properties grid
  const display=after||before;
  h+=`<div class="ac-grid">`;
  if(isUpdate&&before&&after){{
    h+=`<span class="ak">Owner</span>`;
    if(before.owner!==after.owner)h+=`<span class="av changed">${{short(before.owner)}} → ${{short(after.owner)}}</span>`;
    else h+=`<span class="av">${{short(after.owner)}}</span>`;
    h+=`<span class="ak">Lamports</span>`;
    if(before.lamports!==after.lamports){{
      const diff=after.lamports-before.lamports;
      h+=`<span class="av changed">${{before.lamports.toLocaleString()}} → ${{after.lamports.toLocaleString()}} <span style="color:var(${{diff>0?'--g':'--r'}})">(${{diff>0?'+':''}}${{diff.toLocaleString()}})</span></span>`;
    }}else h+=`<span class="av">${{after.lamports.toLocaleString()}}</span>`;
    if(before.space!==undefined){{
      h+=`<span class="ak">Space</span>`;
      if(before.space!==after.space)h+=`<span class="av changed">${{before.space}} → ${{after.space}} bytes</span>`;
      else h+=`<span class="av">${{after.space}} bytes</span>`;
    }}
    h+=`<span class="ak">Executable</span><span class="av">${{after.executable?'Yes':'No'}}</span>`;
  }}else if(display){{
    h+=`<span class="ak">Owner</span><span class="av">${{short(display.owner)}}</span>`;
    h+=`<span class="ak">Lamports</span><span class="av">${{display.lamports.toLocaleString()}}</span>`;
    if(display.space!==undefined)h+=`<span class="ak">Space</span><span class="av">${{display.space}} bytes</span>`;
    h+=`<span class="ak">Executable</span><span class="av">${{display.executable?'Yes':'No'}}</span>`;
  }}
  h+=`</div>`;

  // Parsed JSON data (from jsonParsed encoding)
  const parsedBefore=getParsed(before), parsedAfter=getParsed(after);
  if(parsedAfter||parsedBefore){{
    h+=`<div style="margin-top:10px"><div style="font-size:9px;color:var(--t3);text-transform:uppercase;letter-spacing:.8px;font-family:var(--m);margin-bottom:4px;font-weight:500">Parsed Data</div>`;
    if(isUpdate&&parsedBefore&&parsedAfter){{
      h+=`<div style="display:grid;grid-template-columns:1fr 1fr;gap:8px">`;
      h+=`<div><div style="font-size:9px;color:var(--t3);margin-bottom:2px;font-family:var(--m)">BEFORE</div><div class="json-block">${{syntaxHighlight(JSON.stringify(parsedBefore,null,2))}}</div></div>`;
      h+=`<div><div style="font-size:9px;color:var(--t3);margin-bottom:2px;font-family:var(--m)">AFTER</div><div class="json-block">${{syntaxHighlight(JSON.stringify(parsedAfter,null,2))}}</div></div>`;
      h+=`</div>`;
    }}else{{
      const pd=parsedAfter||parsedBefore;
      h+=`<div class="json-block">${{syntaxHighlight(JSON.stringify(pd,null,2))}}</div>`;
    }}
    h+=`</div>`;
  }}

  // Raw hex data (from base64 encoding, merged via _b64)
  const hexBefore=b64toHex(getB64(before));
  const hexAfter=b64toHex(getB64(after));
  if(hexAfter||hexBefore){{
    h+=`<div style="margin-top:10px"><div style="font-size:9px;color:var(--t3);text-transform:uppercase;letter-spacing:.8px;font-family:var(--m);margin-bottom:4px;font-weight:500">Byte Data</div>`;
    if(isUpdate&&hexBefore&&hexAfter){{
      // Generate diff-highlighted hex dump
      h+=`<div style="display:grid;grid-template-columns:1fr 1fr;gap:8px">`;
      h+=`<div><div style="font-size:9px;color:var(--t3);margin-bottom:2px;font-family:var(--m)">BEFORE (${{hexBefore.length/2}} bytes)</div><div class="hex-block">${{hexDumpDiff(hexBefore,hexAfter,true)}}</div></div>`;
      h+=`<div><div style="font-size:9px;color:var(--t3);margin-bottom:2px;font-family:var(--m)">AFTER (${{hexAfter.length/2}} bytes)</div><div class="hex-block">${{hexDumpDiff(hexAfter,hexBefore,false)}}</div></div>`;
      h+=`</div>`;
    }}else{{
      const hx=hexAfter||hexBefore;
      h+=`<div><div style="font-size:9px;color:var(--t3);margin-bottom:2px;font-family:var(--m)">${{hx.length/2}} bytes</div><div class="hex-block">${{hexDump(hx)}}</div></div>`;
    }}
    h+=`</div>`;
  }}

  h+=`</div></div>`;
  return h;
}}

function syntaxHighlight(json){{
  return esc(json).replace(/"([^"]+)"(?=\s*:)/g,'<span class="jk">"$1"</span>')
    .replace(/: "([^"]*?)"/g,': <span class="js">"$1"</span>')
    .replace(/: (\d+)/g,': <span class="jn">$1</span>');
}}

function logClass(l){{
  if(l.includes('failed')||l.includes('Error'))return' e';
  if(l.includes('Program log:')||l.includes('success'))return' p';
  if(l.includes('Program data:'))return' d';
  return'';
}}

window.showTx=(ii,ti)=>{{
  const tx=R.instances[ii].transactions[ti];
  const jp=tx.profile_json_parsed, b64=tx.profile_base64;
  const p=mergeProfiles(jp,b64);
  mdl.classList.add('open');
  document.getElementById('mt').textContent=tx.signature;
  const cu=p?.transactionProfile?.computeUnitsConsumed||0;

  let h=`<div class="sec">
    <div class="dr"><span class="k">Signature</span><span style="font-family:var(--m);font-size:11px;word-break:break-all">${{tx.signature}}</span></div>
    <div class="dr"><span class="k">Slot</span><span style="font-family:var(--m)">${{tx.slot}}</span></div>
    <div class="dr"><span class="k">Status</span><span style="color:var(${{tx.error?'--r':'--g'}});font-weight:600">${{tx.error?'FAILED':'SUCCESS'}}</span></div>
    ${{tx.error?`<div class="dr"><span class="k">Error</span><span style="color:var(--r);font-family:var(--m);font-size:11px">${{esc(tx.error)}}</span></div>`:''}}
    ${{cu>0?`<div class="dr"><span class="k">Compute Units</span><span style="font-family:var(--m);color:var(--cyan)">${{cu.toLocaleString()}}</span></div>`:''}}
  </div>`;

  if(p){{
    const ixs=p.instructionProfiles||[];
    if(ixs.length>0){{
      // CU breakdown bar
      const totIxCU=ixs.reduce((s,x)=>s+(x.computeUnitsConsumed||0),0);
      h+=`<div class="sec"><h4>Compute Unit Breakdown</h4>`;
      h+=`<div class="cu-bar">${{ixs.map((x,i)=>{{
        const pct=totIxCU>0?(x.computeUnitsConsumed/totIxCU*100):0;
        return`<div class="seg" style="width:${{Math.max(pct,2)}}%;background:${{COLORS[i%COLORS.length]}}" title="Ix ${{i}}: ${{(x.computeUnitsConsumed||0).toLocaleString()}} CU"></div>`;
      }}).join('')}}</div>`;
      h+=`<div style="display:flex;gap:12px;flex-wrap:wrap;margin-top:6px">${{ixs.map((x,i)=>
        `<span style="font-family:var(--m);font-size:10px;color:var(--t3)"><span style="display:inline-block;width:8px;height:8px;border-radius:2px;background:${{COLORS[i%COLORS.length]}};margin-right:4px;vertical-align:middle"></span>Ix ${{i}}: ${{(x.computeUnitsConsumed||0).toLocaleString()}} CU</span>`
      ).join('')}}</div></div>`;

      // Instructions
      h+=`<div class="sec"><h4>Instructions (${{ixs.length}})</h4>`;
      ixs.forEach((ix,i)=>{{
        const accts=Object.entries(ix.accountStates||{{}});
        const writable=accts.filter(([,s])=>s.type==='writable'&&s.accountChange?.type!=='unchanged');
        const program=ix.logMessages?.find(l=>l.match(/^Program \w+ invoke \[1\]/))?.match(/^Program (\w+)/)?.[1]||'';

        h+=`<div class="ix"><div class="ix-h" onclick="this.nextElementSibling.classList.toggle('open')">
          <span><span style="color:${{COLORS[i%COLORS.length]}};margin-right:6px">&#9632;</span>Instruction ${{i}}${{program?` <span style="color:var(--t3);font-size:10px">(${{pgmName(program)}})</span>`:''}}</span>
          <span style="display:flex;gap:8px;align-items:center">
            ${{writable.length>0?`<span style="color:var(--y);font-size:10px">${{writable.length}} changed</span>`:''}}
            <span style="color:var(--t3)">${{(ix.computeUnitsConsumed||0).toLocaleString()}} CU</span>
          </span>
        </div><div class="ix-b${{ixs.length<=3?' open':''}}">`;

        // Logs for this instruction
        if((ix.logMessages||[]).length>0){{
          h+=`<div class="logs" style="margin-bottom:8px">${{ix.logMessages.map(l=>`<div class="ll${{logClass(l)}}">${{esc(l)}}</div>`).join('')}}</div>`;
        }}

        // Account changes
        accts.forEach(([addr,st])=>{{
          h+=renderAccountDetail(addr,st);
        }});

        h+=`</div></div>`;
      }});
      h+=`</div>`;
    }}

    // Readonly accounts
    const ro=p.readonlyAccountStates;
    if(ro&&Object.keys(ro).length>0){{
      h+=`<div class="sec"><h4>Readonly Accounts (${{Object.keys(ro).length}})</h4><div class="ro-section">`;
      for(const[addr,data]of Object.entries(ro)){{
        h+=`<div class="ro-addr"><span>${{addr}}</span><span>${{(data.lamports||0).toLocaleString()}} lamports</span></div>`;
      }}
      h+=`</div></div>`;
    }}
  }}

  // Full program logs
  if(tx.logs.length>0){{
    h+=`<div class="sec"><h4>Program Logs (${{tx.logs.length}})</h4><div class="logs">${{tx.logs.map(l=>`<div class="ll${{logClass(l)}}">${{esc(l)}}</div>`).join('')}}</div></div>`;
  }}

  document.getElementById('mb').innerHTML=h;
}};
}})();
</script>
</body>
</html>"##
    )
}
