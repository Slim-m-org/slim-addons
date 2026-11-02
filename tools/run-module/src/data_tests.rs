use serde_json::{json, Value};

use crate::data::Data;

fn data() -> Data {
    let manifest = json!({"data": {"max_mb": 1, "tables": [{"name": "xp", "columns": [
        {"name": "user_id", "type": "text", "unique": true},
        {"name": "xp", "type": "int", "index": true},
        {"name": "ratio", "type": "real"},
        {"name": "on", "type": "bool"}
    ]}]}});
    Data::from_manifest(&manifest).unwrap()
}

fn call(data: &mut Data, request: Value) -> Value {
    let mut request = request;
    request["capability"] = json!("data.store");
    request["table"] = json!("xp");
    data.handle(&request)
}

fn error(data: &mut Data, request: Value) -> String {
    let reply = call(data, request);
    assert_eq!(reply["ok"], false, "{reply}");
    reply["error"].as_str().unwrap().to_owned()
}

fn seed(data: &mut Data, members: &[(&str, i64)]) {
    let rows: Vec<Value> = members.iter().map(|(u, x)| json!({"user_id": u, "xp": x})).collect();
    assert_eq!(call(data, json!({"op": "insert", "rows": rows}))["ok"], true);
}

fn ids(reply: &Value) -> Vec<&str> {
    reply["rows"].as_array().unwrap().iter().map(|r| r["user_id"].as_str().unwrap()).collect()
}

#[test]
fn insert_numbers_rows_and_a_unique_column_refuses_a_second_claim() {
    let mut d = data();
    let reply = call(&mut d, json!({"op": "insert", "rows": [{"user_id": "a"}, {"user_id": "b"}]}));
    assert_eq!((reply["rows"].clone(), reply["changed"].clone()), (json!([{"_id": 1}, {"_id": 2}]), json!(2)));
    assert!(error(&mut d, json!({"op": "insert", "values": {"user_id": "a"}})).contains("unique column"));
    assert!(error(&mut d, json!({"op": "insert", "rows": [{"user_id": "c"}, {"user_id": "c"}]})).contains("unique column"));
    assert_eq!(call(&mut d, json!({"op": "count"}))["count"], 2);
}

#[test]
fn upsert_adds_in_one_statement_and_returns_the_row() {
    let mut d = data();
    let first = call(&mut d, json!({"op": "upsert", "on": "user_id", "values": {"user_id": "a"}, "add": {"xp": 5}}));
    assert_eq!(first["rows"][0]["xp"], 5);
    let again = call(&mut d, json!({"op": "upsert", "on": "user_id", "values": {"user_id": "a"}, "add": {"xp": 5}}));
    assert_eq!((again["rows"][0]["xp"].clone(), again["rows"][0]["_id"].clone()), (json!(10), json!(1)));
    let untouched = call(&mut d, json!({"op": "upsert", "on": "user_id", "values": {"user_id": "a"}}));
    assert_eq!(untouched["changed"], 0);
    assert!(error(&mut d, json!({"op": "upsert", "on": "xp", "values": {"xp": 1}})).contains("unique"));
    assert!(error(&mut d, json!({"op": "upsert", "on": "user_id", "values": {"user_id": "a", "xp": 1}, "add": {"xp": 1}})).contains("both"));
}

#[test]
fn a_conditional_update_claims_once() {
    let mut d = data();
    seed(&mut d, &[("a", 1)]);
    let claim = json!({"op": "update", "add": {"xp": 10}, "values": {"ratio": 0.5},
        "where": [{"col": "user_id", "op": "eq", "val": "a"}, {"col": "xp", "op": "lt", "val": 5}]});
    assert_eq!(call(&mut d, claim.clone())["changed"], 1);
    assert_eq!(call(&mut d, claim)["changed"], 0);
    assert_eq!(call(&mut d, json!({"op": "get", "where": [{"col": "user_id", "op": "eq", "val": "a"}]}))["rows"][0]["xp"], 11);
}

#[test]
fn query_filters_orders_pages_and_ties_break_by_id() {
    let mut d = data();
    seed(&mut d, &[("a", 5), ("b", 9), ("c", 5), ("d", 1)]);
    let by_xp = call(&mut d, json!({"op": "query", "order": [{"col": "xp", "dir": "desc"}]}));
    assert_eq!(ids(&by_xp), ["b", "a", "c", "d"]);
    let page = call(&mut d, json!({"op": "query", "order": [{"col": "xp"}], "limit": 2, "offset": 1}));
    assert_eq!(ids(&page), ["a", "c"]);
    let some = call(&mut d, json!({"op": "query", "where": [{"col": "user_id", "op": "in", "val": ["a", "d", "zz"]}, {"col": "xp", "op": "ne", "val": 1}]}));
    assert_eq!(ids(&some), ["a"]);
    let like = call(&mut d, json!({"op": "query", "where": [{"col": "user_id", "op": "like", "val": "_"}]}));
    assert_eq!(like["count"], 4);
    assert_eq!(call(&mut d, json!({"op": "get", "order": [{"col": "xp", "dir": "desc"}]}))["count"], 1);
}

#[test]
fn count_sum_increment_and_delete() {
    let mut d = data();
    seed(&mut d, &[("a", 5), ("b", 9)]);
    assert_eq!(call(&mut d, json!({"op": "sum", "col": "xp"}))["count"], 14);
    assert_eq!(call(&mut d, json!({"op": "increment", "col": "xp", "by": 3, "where": [{"col": "xp", "op": "gt", "val": 6}]}))["changed"], 1);
    assert_eq!(call(&mut d, json!({"op": "increment", "col": "ratio", "by": 0.5}))["changed"], 2);
    assert_eq!(call(&mut d, json!({"op": "sum", "col": "xp"}))["count"], 17);
    assert_eq!(call(&mut d, json!({"op": "delete", "where": [{"col": "xp", "op": "gte", "val": 12}]}))["changed"], 1);
    assert_eq!(call(&mut d, json!({"op": "count"}))["count"], 1);
}

#[test]
fn the_hosts_refusals_are_the_runners() {
    let mut d = data();
    assert!(error(&mut d, json!({"op": "insert", "values": {"nope": 1}})).contains("unknown column"));
    assert!(error(&mut d, json!({"op": "insert", "values": {"xp": "many"}})).contains("column type"));
    assert!(error(&mut d, json!({"op": "insert", "values": {"_id": 4}})).contains("_id"));
    assert!(error(&mut d, json!({"op": "query", "where": [{"col": "xp", "op": "eq", "val": null}]})).contains("null"));
    assert!(error(&mut d, json!({"op": "query", "limit": 201})).contains("limit"));
    assert!(error(&mut d, json!({"op": "query", "offset": 10_001})).contains("offset"));
    assert!(error(&mut d, json!({"op": "query", "where": [{"col": "xp", "op": "like", "val": "1"}]})).contains("operator"));
    assert!(error(&mut d, json!({"op": "insert", "values": {"user_id": "x".repeat(4097)}})).contains("too large"));
    assert!(error(&mut d, json!({"op": "insert", "values": {"xp": 1}, "add": {"xp": 1}})).contains("add is for"));
    assert!(error(&mut d, json!({"op": "drop"})).contains("unknown data.store op"));
    assert_eq!(d.handle(&json!({"op": "count", "table": "other"}))["error"], "unknown table");
}

#[test]
fn a_file_at_its_cap_refuses_writes_and_state_survives_a_dump() {
    let mut d = data();
    let big = "x".repeat(4000);
    let mut refused = false;
    for n in 0..400 {
        let reply = call(&mut d, json!({"op": "insert", "values": {"user_id": format!("{n}{big}")}}));
        if reply["error"] == "data.store is full for this module" {
            refused = true;
            break;
        }
    }
    assert!(refused);
    let mut fresh = data();
    fresh.load(&d.dump());
    assert_eq!(call(&mut fresh, json!({"op": "count"}))["count"], call(&mut d, json!({"op": "count"}))["count"]);
    call(&mut fresh, json!({"op": "delete"}));
    let next = call(&mut fresh, json!({"op": "insert", "values": {"user_id": "later"}}));
    assert!(next["rows"][0]["_id"].as_i64().unwrap() > 1);
}
