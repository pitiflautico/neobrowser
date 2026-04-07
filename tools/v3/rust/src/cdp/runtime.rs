//! CDP Runtime domain — typed wrappers for Runtime.* methods.
//!
//! Every function takes `&dyn CdpTransport` as first arg so it works
//! with both real CdpSession and MockTransport.

use super::{CdpTransport, CdpResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// ── Types ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteObject {
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtype: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unserializable_value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExceptionDetails {
    pub exception_id: i64,
    pub text: String,
    pub line_number: i64,
    pub column_number: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exception: Option<RemoteObject>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stack_trace: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluateResult {
    pub result: RemoteObject,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exception_details: Option<ExceptionDetails>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluateParams {
    pub expression: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_group: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_command_line_api: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub silent: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_by_value: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generate_preview: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_gesture: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub await_promise: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub throw_on_side_effect: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unique_context_id: Option<String>,
}

impl EvaluateParams {
    pub fn new(expression: impl Into<String>) -> Self {
        Self {
            expression: expression.into(),
            object_group: None,
            include_command_line_api: None,
            silent: None,
            context_id: None,
            return_by_value: None,
            generate_preview: None,
            user_gesture: None,
            await_promise: None,
            throw_on_side_effect: None,
            timeout: None,
            unique_context_id: None,
        }
    }

    pub fn await_promise(mut self) -> Self {
        self.await_promise = Some(true);
        self
    }

    pub fn return_by_value(mut self) -> Self {
        self.return_by_value = Some(true);
        self
    }

    pub fn context_id(mut self, id: i64) -> Self {
        self.context_id = Some(id);
        self
    }

    pub fn silent(mut self) -> Self {
        self.silent = Some(true);
        self
    }

    pub fn timeout_ms(mut self, ms: f64) -> Self {
        self.timeout = Some(ms);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallFunctionOnParams {
    pub function_declaration: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Vec<CallArgument>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub silent: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_by_value: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generate_preview: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_gesture: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub await_promise: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_context_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_group: Option<String>,
}

impl CallFunctionOnParams {
    pub fn new(function_declaration: impl Into<String>) -> Self {
        Self {
            function_declaration: function_declaration.into(),
            object_id: None,
            arguments: None,
            silent: None,
            return_by_value: None,
            generate_preview: None,
            user_gesture: None,
            await_promise: None,
            execution_context_id: None,
            object_group: None,
        }
    }

    pub fn object_id(mut self, id: impl Into<String>) -> Self {
        self.object_id = Some(id.into());
        self
    }

    pub fn arg_value(mut self, val: Value) -> Self {
        let arg = CallArgument {
            value: Some(val),
            unserializable_value: None,
            object_id: None,
        };
        self.arguments.get_or_insert_with(Vec::new).push(arg);
        self
    }

    pub fn await_promise(mut self) -> Self {
        self.await_promise = Some(true);
        self
    }

    pub fn return_by_value(mut self) -> Self {
        self.return_by_value = Some(true);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallArgument {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unserializable_value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PropertyDescriptor {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<RemoteObject>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub writable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub get: Option<RemoteObject>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub set: Option<RemoteObject>,
    pub configurable: bool,
    pub enumerable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub was_thrown: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_own: Option<bool>,
}

// ── Events ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleApiCalledEvent {
    #[serde(rename = "type")]
    pub type_: String,
    pub args: Vec<RemoteObject>,
    pub execution_context_id: i64,
    pub timestamp: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stack_trace: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExceptionThrownEvent {
    pub timestamp: f64,
    pub exception_details: ExceptionDetails,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionContextCreatedEvent {
    pub context: ExecutionContextDescription,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionContextDescription {
    pub id: i64,
    pub origin: String,
    pub name: String,
    pub unique_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aux_data: Option<Value>,
}

// ── Methods ─────────────────────────────────────────────────────────

pub async fn enable(transport: &dyn CdpTransport) -> CdpResult<()> {
    transport.send("Runtime.enable", json!(null)).await?;
    Ok(())
}

pub async fn disable(transport: &dyn CdpTransport) -> CdpResult<()> {
    transport.send("Runtime.disable", json!(null)).await?;
    Ok(())
}

pub async fn evaluate(
    transport: &dyn CdpTransport,
    params: EvaluateParams,
) -> CdpResult<EvaluateResult> {
    let params_value = serde_json::to_value(&params)?;
    let resp = transport.send("Runtime.evaluate", params_value).await?;
    let result: EvaluateResult = serde_json::from_value(resp)?;
    Ok(result)
}

pub async fn call_function_on(
    transport: &dyn CdpTransport,
    params: CallFunctionOnParams,
) -> CdpResult<EvaluateResult> {
    let params_value = serde_json::to_value(&params)?;
    let resp = transport.send("Runtime.callFunctionOn", params_value).await?;
    let result: EvaluateResult = serde_json::from_value(resp)?;
    Ok(result)
}

pub async fn get_properties(
    transport: &dyn CdpTransport,
    object_id: &str,
    own_properties: Option<bool>,
    accessor_properties_only: Option<bool>,
    generate_preview: Option<bool>,
) -> CdpResult<Vec<PropertyDescriptor>> {
    let mut params = json!({ "objectId": object_id });
    if let Some(v) = own_properties {
        params["ownProperties"] = json!(v);
    }
    if let Some(v) = accessor_properties_only {
        params["accessorPropertiesOnly"] = json!(v);
    }
    if let Some(v) = generate_preview {
        params["generatePreview"] = json!(v);
    }
    let resp = transport.send("Runtime.getProperties", params).await?;
    let descriptors: Vec<PropertyDescriptor> = serde_json::from_value(
        resp.get("result").cloned().unwrap_or(json!([])),
    )?;
    Ok(descriptors)
}

pub async fn release_object(
    transport: &dyn CdpTransport,
    object_id: &str,
) -> CdpResult<()> {
    transport
        .send("Runtime.releaseObject", json!({ "objectId": object_id }))
        .await?;
    Ok(())
}

pub async fn release_object_group(
    transport: &dyn CdpTransport,
    object_group: &str,
) -> CdpResult<()> {
    transport
        .send("Runtime.releaseObjectGroup", json!({ "objectGroup": object_group }))
        .await?;
    Ok(())
}

pub async fn add_binding(
    transport: &dyn CdpTransport,
    name: &str,
    execution_context_id: Option<i64>,
    execution_context_name: Option<&str>,
) -> CdpResult<()> {
    let mut params = json!({ "name": name });
    if let Some(id) = execution_context_id {
        params["executionContextId"] = json!(id);
    }
    if let Some(n) = execution_context_name {
        params["executionContextName"] = json!(n);
    }
    transport.send("Runtime.addBinding", params).await?;
    Ok(())
}

pub async fn remove_binding(
    transport: &dyn CdpTransport,
    name: &str,
) -> CdpResult<()> {
    transport
        .send("Runtime.removeBinding", json!({ "name": name }))
        .await?;
    Ok(())
}

pub async fn discard_console_entries(transport: &dyn CdpTransport) -> CdpResult<()> {
    transport
        .send("Runtime.discardConsoleEntries", json!(null))
        .await?;
    Ok(())
}

pub async fn run_if_waiting_for_debugger(transport: &dyn CdpTransport) -> CdpResult<()> {
    transport
        .send("Runtime.runIfWaitingForDebugger", json!(null))
        .await?;
    Ok(())
}

pub async fn compile_script(
    transport: &dyn CdpTransport,
    expression: &str,
    source_url: &str,
    persist_script: bool,
    execution_context_id: Option<i64>,
) -> CdpResult<Value> {
    let mut params = json!({
        "expression": expression,
        "sourceURL": source_url,
        "persistScript": persist_script,
    });
    if let Some(id) = execution_context_id {
        params["executionContextId"] = json!(id);
    }
    let resp = transport.send("Runtime.compileScript", params).await?;
    Ok(resp)
}

pub async fn run_script(
    transport: &dyn CdpTransport,
    script_id: &str,
    execution_context_id: Option<i64>,
    await_promise: Option<bool>,
) -> CdpResult<EvaluateResult> {
    let mut params = json!({ "scriptId": script_id });
    if let Some(id) = execution_context_id {
        params["executionContextId"] = json!(id);
    }
    if let Some(v) = await_promise {
        params["awaitPromise"] = json!(v);
    }
    let resp = transport.send("Runtime.runScript", params).await?;
    let result: EvaluateResult = serde_json::from_value(resp)?;
    Ok(result)
}

pub async fn get_heap_usage(transport: &dyn CdpTransport) -> CdpResult<Value> {
    let resp = transport.send("Runtime.getHeapUsage", json!(null)).await?;
    Ok(resp)
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdp::MockTransport;
    use serde_json::json;

    #[tokio::test]
    async fn test_enable() {
        let mock = MockTransport::new();
        mock.expect("Runtime.enable", json!({})).await;

        enable(&mock).await.unwrap();
        mock.assert_called_once("Runtime.enable").await;
    }

    #[tokio::test]
    async fn test_evaluate_simple() {
        let mock = MockTransport::new();
        mock.expect(
            "Runtime.evaluate",
            json!({
                "result": {
                    "type": "number",
                    "value": 2,
                    "description": "2"
                }
            }),
        )
        .await;

        let params = EvaluateParams::new("1+1").return_by_value();
        let res = evaluate(&mock, params).await.unwrap();

        assert_eq!(res.result.type_, "number");
        assert_eq!(res.result.value, Some(json!(2)));
        assert!(res.exception_details.is_none());

        let sent = mock.call_params("Runtime.evaluate", 0).await.unwrap();
        assert_eq!(sent["expression"], "1+1");
        assert_eq!(sent["returnByValue"], true);
    }

    #[tokio::test]
    async fn test_evaluate_await_promise() {
        let mock = MockTransport::new();
        mock.expect(
            "Runtime.evaluate",
            json!({
                "result": {
                    "type": "string",
                    "value": "resolved"
                }
            }),
        )
        .await;

        let params = EvaluateParams::new("Promise.resolve('resolved')")
            .await_promise()
            .return_by_value();
        let res = evaluate(&mock, params).await.unwrap();

        assert_eq!(res.result.value, Some(json!("resolved")));

        let sent = mock.call_params("Runtime.evaluate", 0).await.unwrap();
        assert_eq!(sent["awaitPromise"], true);
        assert_eq!(sent["returnByValue"], true);
    }

    #[tokio::test]
    async fn test_evaluate_with_exception() {
        let mock = MockTransport::new();
        mock.expect(
            "Runtime.evaluate",
            json!({
                "result": {
                    "type": "object",
                    "subtype": "error",
                    "className": "ReferenceError",
                    "description": "ReferenceError: foo is not defined"
                },
                "exceptionDetails": {
                    "exceptionId": 1,
                    "text": "Uncaught",
                    "lineNumber": 0,
                    "columnNumber": 0,
                    "exception": {
                        "type": "object",
                        "subtype": "error",
                        "className": "ReferenceError",
                        "description": "ReferenceError: foo is not defined"
                    }
                }
            }),
        )
        .await;

        let params = EvaluateParams::new("foo");
        let res = evaluate(&mock, params).await.unwrap();

        assert!(res.exception_details.is_some());
        let exc = res.exception_details.unwrap();
        assert_eq!(exc.exception_id, 1);
        assert_eq!(exc.text, "Uncaught");
        assert_eq!(exc.exception.unwrap().class_name, Some("ReferenceError".into()));
    }

    #[tokio::test]
    async fn test_evaluate_params_builder() {
        let params = EvaluateParams::new("document.title")
            .await_promise()
            .return_by_value()
            .context_id(42)
            .silent()
            .timeout_ms(5000.0);

        assert_eq!(params.expression, "document.title");
        assert_eq!(params.await_promise, Some(true));
        assert_eq!(params.return_by_value, Some(true));
        assert_eq!(params.context_id, Some(42));
        assert_eq!(params.silent, Some(true));
        assert_eq!(params.timeout, Some(5000.0));
        // Unset fields are None
        assert!(params.object_group.is_none());
        assert!(params.generate_preview.is_none());
        assert!(params.unique_context_id.is_none());
    }

    #[tokio::test]
    async fn test_call_function_on() {
        let mock = MockTransport::new();
        mock.expect(
            "Runtime.callFunctionOn",
            json!({
                "result": {
                    "type": "string",
                    "value": "hello world"
                }
            }),
        )
        .await;

        let params = CallFunctionOnParams::new("function(a, b) { return a + ' ' + b; }")
            .object_id("obj-123")
            .arg_value(json!("hello"))
            .arg_value(json!("world"))
            .return_by_value();

        let res = call_function_on(&mock, params).await.unwrap();
        assert_eq!(res.result.value, Some(json!("hello world")));

        let sent = mock.call_params("Runtime.callFunctionOn", 0).await.unwrap();
        assert_eq!(sent["objectId"], "obj-123");
        assert_eq!(sent["arguments"].as_array().unwrap().len(), 2);
        assert_eq!(sent["returnByValue"], true);
    }

    #[tokio::test]
    async fn test_call_function_on_builder() {
        let params = CallFunctionOnParams::new("function() {}")
            .object_id("node-1")
            .arg_value(json!(42))
            .arg_value(json!("test"))
            .await_promise()
            .return_by_value();

        assert_eq!(params.function_declaration, "function() {}");
        assert_eq!(params.object_id, Some("node-1".into()));
        assert_eq!(params.await_promise, Some(true));
        assert_eq!(params.return_by_value, Some(true));
        let args = params.arguments.unwrap();
        assert_eq!(args.len(), 2);
        assert_eq!(args[0].value, Some(json!(42)));
        assert_eq!(args[1].value, Some(json!("test")));
    }

    #[tokio::test]
    async fn test_get_properties() {
        let mock = MockTransport::new();
        mock.expect(
            "Runtime.getProperties",
            json!({
                "result": [
                    {
                        "name": "length",
                        "value": { "type": "number", "value": 5 },
                        "configurable": true,
                        "enumerable": false,
                        "writable": true,
                        "isOwn": true
                    },
                    {
                        "name": "toString",
                        "value": { "type": "function", "description": "function toString() { [native code] }" },
                        "configurable": true,
                        "enumerable": false
                    }
                ]
            }),
        )
        .await;

        let props = get_properties(&mock, "obj-1", Some(true), None, None).await.unwrap();
        assert_eq!(props.len(), 2);
        assert_eq!(props[0].name, "length");
        assert_eq!(props[0].is_own, Some(true));
        assert_eq!(props[1].name, "toString");
        assert!(props[1].is_own.is_none());

        let sent = mock.call_params("Runtime.getProperties", 0).await.unwrap();
        assert_eq!(sent["objectId"], "obj-1");
        assert_eq!(sent["ownProperties"], true);
    }

    #[tokio::test]
    async fn test_release_object() {
        let mock = MockTransport::new();
        mock.expect("Runtime.releaseObject", json!({})).await;

        release_object(&mock, "obj-42").await.unwrap();

        let sent = mock.call_params("Runtime.releaseObject", 0).await.unwrap();
        assert_eq!(sent["objectId"], "obj-42");
    }

    #[tokio::test]
    async fn test_add_binding() {
        let mock = MockTransport::new();
        mock.expect("Runtime.addBinding", json!({})).await;

        add_binding(&mock, "myCallback", Some(1), None).await.unwrap();

        let sent = mock.call_params("Runtime.addBinding", 0).await.unwrap();
        assert_eq!(sent["name"], "myCallback");
        assert_eq!(sent["executionContextId"], 1);
        assert!(sent.get("executionContextName").is_none());
    }

    #[tokio::test]
    async fn test_discard_console_entries() {
        let mock = MockTransport::new();
        mock.expect("Runtime.discardConsoleEntries", json!({})).await;

        discard_console_entries(&mock).await.unwrap();
        mock.assert_called_once("Runtime.discardConsoleEntries").await;
    }

    #[tokio::test]
    async fn test_compile_and_run_script() {
        let mock = MockTransport::new();
        mock.expect(
            "Runtime.compileScript",
            json!({ "scriptId": "script-1" }),
        )
        .await;
        mock.expect(
            "Runtime.runScript",
            json!({
                "result": {
                    "type": "number",
                    "value": 42
                }
            }),
        )
        .await;

        // Compile
        let compiled = compile_script(&mock, "21 * 2", "test.js", true, None)
            .await
            .unwrap();
        assert_eq!(compiled["scriptId"], "script-1");

        let sent = mock.call_params("Runtime.compileScript", 0).await.unwrap();
        assert_eq!(sent["expression"], "21 * 2");
        assert_eq!(sent["sourceURL"], "test.js");
        assert_eq!(sent["persistScript"], true);

        // Run
        let script_id = compiled["scriptId"].as_str().unwrap();
        let res = run_script(&mock, script_id, None, Some(true)).await.unwrap();
        assert_eq!(res.result.value, Some(json!(42)));

        let sent = mock.call_params("Runtime.runScript", 0).await.unwrap();
        assert_eq!(sent["scriptId"], "script-1");
        assert_eq!(sent["awaitPromise"], true);
    }
}
