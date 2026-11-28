mod support;

use aaqe::{
    audit::{
        harness::AuditHarness,
        interfaces::AuditAgent,
        workflow::{reproduce, run_audit_with},
    },
    config::Config,
    runtime::{compose::ComposeEnvironment, process::run},
    state::{Store, read_json},
};
use anyhow::Result;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

struct DatabaseAgent {
    inventory: Value,
}

const CHECK: &str = "import urllib.request\nwith urllib.request.urlopen('http://influxdb:8086/ping',timeout=5) as r:\n assert r.status==204\n print(r.headers['X-Influxdb-Version'])";

fn scripts() -> [&'static str; 3] {
    [
        "import urllib.request,json\nwith urllib.request.urlopen(urllib.request.Request('http://influxdb:8086/query?q=CREATE+DATABASE+aaqe_product',data=b''),timeout=5) as r:\n assert not json.load(r)['results'][0].get('error')\nwith urllib.request.urlopen(urllib.request.Request('http://influxdb:8086/write?db=aaqe_product&precision=ns',data=b'points value=42i 1767225600000000001'),timeout=5) as r:\n assert r.status==204\n print('write accepted')",
        "import urllib.request,json\nwith urllib.request.urlopen('http://influxdb:8086/query?db=aaqe_product&q=SELECT+value+FROM+points',timeout=5) as r:\n point=json.load(r)['results'][0]['series'][0]['values'][0]\n assert point==['2026-01-01T00:00:00.000000001Z',42],point\n print('nanosecond preserved')",
        "import urllib.request,urllib.error,json\ntry:\n urllib.request.urlopen('http://influxdb:8086/query?q=SELECT+FROM',timeout=5)\nexcept urllib.error.HTTPError as error:\n assert error.code==400,error.code\n result=json.load(error)\n assert result.get('error'),result\n print('invalid SQL rejected')\nelse:\n raise AssertionError('invalid SQL was accepted')",
    ]
}

impl AuditAgent for DatabaseAgent {
    fn product_workflow(&self) -> bool {
        true
    }

    fn coordinate(&self, harness: &mut AuditHarness<'_>) -> Result<()> {
        for stage in aaqe::agents::stage::PRODUCT_STAGES {
            let result = harness.run_stage(stage, "通过真实 InfluxDB 验证产品操作与证据")?;
            anyhow::ensure!(result["status"] == "completed", "{result}");
        }
        harness.finish(
            "completed",
            "正常写入、纳秒边界和异常 SQL 已实际验证并独立复核",
        )
    }

    fn invoke(
        &self,
        stage: &str,
        _: &Path,
        directory: &Path,
        inputs: &Value,
        _: &dyn Fn(&str),
    ) -> Result<Value> {
        Ok(match stage {
            "product-explorer" => self.inventory.clone(),
            "scenario-designer" => {
                let scenarios = [
                    ("write", "正常写入", "api", "normal", "创建数据库，通过写入 API 写入带纳秒时间戳的整数数据", "HTTP 204"),
                    ("precision", "纳秒时间戳边界", "sql", "boundary", "通过查询接口执行 SELECT value FROM points", "时间为 2026-01-01T00:00:00.000000001Z，值为 42"),
                    ("invalid", "异常 SQL", "sql", "abnormal", "提交缺少表达式的 SELECT FROM", "返回 SQL 错误，不能作为有效查询执行"),
                ].into_iter().map(|(id,title,entry,category,action,expected)| json!({
                    "id":id,"title":title,"feature":"InfluxDB 读写与查询","entry_point":entry,"category":category,
                    "preconditions":["真实 InfluxDB 1.8 服务可访问；边界查询在写入完成后进行"],"services":["database"],
                    "oracle":expected,"steps":[{"id":"action","action":action,"expected":expected}],"cleanup":["删除本测试创建的 Docker 资源"],
                    "blocked_reason":null,"derived_from":null,"reason":null
                })).collect::<Vec<_>>();
                json!({"summary":"先保存正常、边界和异常操作","scenarios":scenarios,"coverage":[
                    {"feature":"写入","entry_point":"api","category":"normal","status":"planned","rationale":"正常 API 写入","scenario_ids":["write"]},
                    {"feature":"精度","entry_point":"sql","category":"boundary","status":"planned","rationale":"纳秒不可丢失","scenario_ids":["precision"]},
                    {"feature":"解析","entry_point":"sql","category":"abnormal","status":"planned","rationale":"无效 SQL 被拒绝","scenario_ids":["invalid"]}
                ],"limitations":["此集成验证仅覆盖数据库 API 与 SQL；未覆盖 Explorer 页面或 CLI"]})
            }
            "product-executor" => {
                assert!(
                    directory
                        .ancestors()
                        .nth(2)
                        .unwrap()
                        .join("steps.md")
                        .is_file()
                );
                let operations = ["write","precision","invalid"].into_iter().zip(scripts()).map(|(id,script)| json!({
                    "id":id,"scenario_id":id,"step_id":"action","command":["python3","-c",script],"purpose":"action"
                })).collect::<Vec<_>>();
                json!({"summary":"在真实数据库执行已保存操作","operations":operations,"observations":[],"limitations":[]})
            }
            "issue-reproducer" => {
                json!({"summary":"没有观察到产品缺陷","operations":[],"observations":[],"limitations":[],"issues":[],"followups":[]})
            }
            "result-reviewer" => {
                let results = ["write","precision","invalid"].into_iter().zip(["write accepted","nanosecond preserved","invalid SQL rejected"]).map(|(id,expected)| {
                    let record = inputs["execution"]["operations"].as_array().unwrap().iter().find(|r| r["scenario_id"] == id).unwrap();
                    eprintln!("{id}: exit={}, stdout={}, stderr={}", record["result"]["returncode"], record["result"]["stdout"], record["result"]["stderr"]);
                    let passed = record["result"]["returncode"] == 0 && record["result"]["stdout"].as_str().unwrap().contains(expected);
                    json!({"scenario_id":id,"status":if passed {"passed"} else {"failed"},"rationale":format!("检查真实返回结果：{expected}"),
                        "evidence":[Path::new(record["directory"].as_str().unwrap()).join("execution.json")],
                        "assessment":{"executed":true,"expectation_met":passed,"infrastructure_failure":false,"no_mocks":true,"real_services_verified":true,"entry_point_verified":true}})
                }).collect::<Vec<_>>();
                json!({"summary":"根据原始响应逐场景复核","scenario_results":results,"issue_results":[],"limitations":[]})
            }
            _ => unreachable!(),
        })
    }
}

struct Cleanup(PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        if let Ok(entries) = std::fs::read_dir(&self.0) {
            // Remove the external runner before its separately owned database.
            let mut states = entries
                .flatten()
                .filter_map(|e| read_json(&e.path().join("run.json")).ok())
                .collect::<Vec<_>>();
            states.reverse();
            for state in states {
                if let Some(root) = state["workspace"]["path"].as_str() {
                    let _ = ComposeEnvironment::new(
                        Path::new(root),
                        &self.0.join(state["id"].as_str().unwrap()),
                        state["id"].as_str().unwrap(),
                        state["environment"].clone(),
                        60,
                    )
                    .cleanup();
                }
            }
        }
    }
}

#[test]
#[ignore = "requires Docker with a real InfluxDB image"]
fn real_product_scenarios_docker_external_replay_and_unavailable_endpoint() {
    let temp = tempfile::tempdir().unwrap();
    let root = support::repository(temp.path(), false);
    let config = Config {
        state_dir: temp.path().join("runs"),
        docker_timeout: 120,
        ..Config::default()
    };
    let _cleanup = Cleanup(config.state_dir.clone());
    let mut environment = json!({"compose_files":[],"services":["runner","influxdb"],"runner_service":"runner","runner_workdir":"/workspace","external_containers":[],"readiness_timeout":60,
        "compose":{"services":{
            "runner":{"image":"python:3.12-slim","command":["sleep","infinity"],"depends_on":["influxdb"]},
            "influxdb":{"image":"influxdb:1.8.10"}
        }},"readiness":[["python3","-c","import urllib.request,time\nfor attempt in range(30):\n try:\n  urllib.request.urlopen('http://influxdb:8086/ping',timeout=2).close()\n  break\n except Exception:\n  if attempt==29: raise\n  time.sleep(1)\n"]],"setup":[]});
    let inventory = |environment: Value, external: bool| {
        json!({
            "summary":"验证真实 InfluxDB 产品接口","scope":["InfluxDB 读写与查询"],"references":[],"uncertainties":[],
            "entry_points":[{"id":"sql","kind":"sql","description":"查询 SQL","applicable":true,"reason":"提供查询接口"},{"id":"api","kind":"api","description":"写入 API","applicable":true,"reason":"提供 HTTP API"}],
            "services":[{"id":"database","name":"InfluxDB","source":if external {"external"} else {"docker"},"endpoint":if external {json!("http://influxdb:8086")} else {Value::Null},"compose_service":if external {Value::Null} else {json!("influxdb")},"version":null,"check":["python3","-c",CHECK]}],"environment":environment
        })
    };
    let execute = |prompt: &str, inventory: Value| {
        run_audit_with(
            prompt,
            &root,
            &config,
            &DatabaseAgent { inventory },
            &|root, directory, id, spec, timeout| {
                Box::new(ComposeEnvironment::new(root, directory, id, spec, timeout))
            },
            &|message| eprintln!("{message}"),
        )
    };
    let docker = execute(
        "验证 InfluxDB 读写与查询",
        inventory(environment.clone(), false),
    )
    .unwrap();
    assert_eq!(docker["status"], "completed", "{docker}");
    let docker_directory = config.state_dir.join(docker["id"].as_str().unwrap());
    let report = read_json(&docker_directory.join("report.json")).unwrap();
    assert!(
        report["scenario_results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["status"] == "passed")
    );
    assert!(report["execution"]["coverage"].is_null());
    assert!(report["publication"].is_null());
    assert!(
        report["execution"]["operations"][0]["service_checks"][0]["docker"]["containers"][0]["id"]
            .is_string()
    );

    // The second run uses the first run's explicitly supplied external service.
    // Its Compose file includes a replacement via depends_on: it must stay down.
    let owner = ComposeEnvironment::new(
        Path::new(docker["workspace"]["path"].as_str().unwrap()),
        &docker_directory,
        docker["id"].as_str().unwrap(),
        docker["environment"].clone(),
        60,
    );
    environment["compose"]["networks"] =
        json!({"default":{"external":true,"name":format!("{}_default",owner.project)}});
    environment["readiness"] = json!([]);
    let mut external = execute(
        "验证 InfluxDB，服务地址 http://influxdb:8086",
        inventory(environment, true),
    )
    .unwrap();
    assert_eq!(external["status"], "completed", "{external}");
    let external_directory = config.state_dir.join(external["id"].as_str().unwrap());
    let runtime = ComposeEnvironment::new(
        Path::new(external["workspace"]["path"].as_str().unwrap()),
        &external_directory,
        external["id"].as_str().unwrap(),
        external["environment"].clone(),
        60,
    );
    let mut ps = runtime.arguments(true).unwrap();
    ps.extend(["ps".into(), "--all".into(), "-q".into(), "influxdb".into()]);
    assert_eq!(
        run(&ps, None, 30, None).stdout.trim(),
        "",
        "external target got a Docker replacement"
    );
    let store = Store::new(&config.state_dir);
    let original = read_json(&external_directory.join("report.json")).unwrap();
    let replay = reproduce(&store, &mut external, Some("precision")).unwrap();
    assert_eq!(replay.as_array().unwrap().len(), 1);
    assert!(
        replay[0]["result"]["stdout"]
            .as_str()
            .unwrap()
            .contains("nanosecond preserved")
    );
    assert_eq!(replay[0]["service_checks"][0]["source"], "external");
    assert_eq!(
        read_json(&external_directory.join("report.json")).unwrap()["scenario_results"],
        original["scenario_results"]
    );

    let mut stop = owner.arguments(true).unwrap();
    stop.extend(["stop".into(), "influxdb".into()]);
    assert_eq!(run(&stop, None, 30, None).returncode, 0);
    assert!(reproduce(&store, &mut external, Some("precision")).is_err());
    let attempt = external["reproductions"]
        .as_array()
        .unwrap()
        .last()
        .unwrap();
    assert_eq!(attempt["status"], "failed");
    assert_eq!(attempt["records"][0]["target_executed"], false);
    assert_eq!(
        run(&ps, None, 30, None).stdout.trim(),
        "",
        "unavailable explicit endpoint got a local replacement"
    );
}
