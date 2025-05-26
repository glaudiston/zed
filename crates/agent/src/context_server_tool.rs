use std::sync::Arc;

use anyhow::{Result, anyhow, bail};
use assistant_settings::AssistantSettings;
use assistant_tool::{
    ActionLog, Tool, ToolResult, ToolResultContent as AssistantToolResultContent,
    ToolResultOutput, ToolSource,
};
use settings::Settings;
use context_server::{ContextServerId, types};
use gpui::{AnyWindowHandle, App, Entity, Size, Task};
use language_model::{
    LanguageModel, LanguageModelImage, LanguageModelRequest,
    LanguageModelToolResultContent as LmToolResultContent, LanguageModelToolSchemaFormat,
};
use project::{Project, context_server_store::ContextServerStore};
use ui::IconName;

pub struct ContextServerTool {
    store: Entity<ContextServerStore>,
    server_id: ContextServerId,
    tool: types::Tool,
}

impl ContextServerTool {
    pub fn new(
        store: Entity<ContextServerStore>,
        server_id: ContextServerId,
        tool: types::Tool,
    ) -> Self {
        Self {
            store,
            server_id,
            tool,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use assistant_settings::AssistantSettings;
    use context_server::transport::Transport;
    use context_server::{self as cx_server_types, ContextServer}; // aliasing to avoid conflict
    use gpui::{TestAppContext, AppContext as _, Model, Subscription};
    use project::project_settings::{
        ContextServerConfiguration as ProjectContextServerConfig, ProjectSettings,
        ZedToolConfirmationSettings,
    };
    use project::worktree::{Worktree, WorktreeId};
    use project::{self, FakeFs, ProjectPath};
    use settings::{Settings, SettingsStore};
    use smol::stream::StreamExt;

    // Helper to initialize settings for tests
    fn init_test_app_settings(cx: &mut TestAppContext) {
        cx.update(|cx| {
            SettingsStore::test(cx);
            project::Project::init_settings(cx);
            AssistantSettings::register(cx);
            // Ensure ProjectSettings are initialized if not already part of Project::init_settings
            ProjectSettings::register(cx);
        });
    }

    struct TestSetup {
        project: Model<Project>,
        store: Model<ContextServerStore>,
        server_id: ContextServerId,
        cx: TestAppContext,
    }

    // Helper to build a basic test server configuration
    fn mcp_tool(name: &str) -> cx_server_types::types::Tool {
        cx_server_types::types::Tool {
            name: name.to_string(),
            description: Some(format!("Description for {}", name)),
            input_schema: serde_json::json!({"type": "object"}),
        }
    }

    // Mock transport for ContextServer
    // This is needed because ContextServerStore tries to start/manage actual servers
    // which involves transport layer. For these tests, we don't need real server communication.
    struct MockTransport;
    #[async_trait::async_trait]
    impl Transport for MockTransport {
        async fn send(&self, _message: String) -> anyhow::Result<()> {
            Ok(())
        }
        fn receive(
            &self,
        ) -> smol::stream::Receiver<Result<String, project::context_server::ConnectionError>>
        {
            let (_tx, rx) = smol::channel::unbounded();
            rx
        }
        fn clone_boxed(&self) -> Box<dyn Transport + Send + Sync> {
            Box::new(MockTransport)
        }
         fn status(&self) -> project::context_server::StatusStream {
            let (_tx, rx) = smol::channel::unbounded();
            rx
        }
        fn request_count(&self) -> Arc<AtomicUsize> {
            Arc::new(AtomicUsize::new(0))
        }
    }


    async fn setup_environment(
        cx: &mut TestAppContext,
        server_id_str: &str,
        confirmation_settings: Option<ZedToolConfirmationSettings>,
    ) -> TestSetup {
        init_test_app_settings(cx);

        let fs = FakeFs::new(cx.executor());
        let project = Project::test(fs, [], cx).await;
        let worktree_store = project.read_with(cx, |p, c| p.worktree_store().clone());

        // Create ContextServerStore without the maintain_server_loop for simpler test setup
        // We will manually manage the server state for this test.
        let store_registry =
            project::context_server_store::registry::ContextServerDescriptorRegistry::default_global(cx);

        let store = cx.new_model(|cx_model| {
            let mut css = ContextServerStore::test(store_registry, worktree_store.clone(), cx_model);
            // Manually insert a server configuration for ContextServerStore to find
            let server_id = ContextServerId(server_id_str.into());
            let mut project_config = ProjectContextServerConfig::default();
            project_config.zed_tool_confirmation = confirmation_settings;
            project_config.command = Some(cx_server_types::ContextServerCommand { // Dummy command
                path: "dummy_server_path".to_string(),
                args: vec![],
                env: None,
            });


            // ContextServerStore uses `maintain_servers` to populate from settings.
            // For testing `get_confirmation_settings` directly, we need to ensure
            // the `servers` map in `ContextServerStore` has an entry for our `server_id`
            // with the correct `ProjectContextServerConfig`.
            // We can achieve this by setting project settings and then calling `available_context_servers_changed`.
            // This is closer to "Option A".

            cx_model.update_global::<SettingsStore, _>(|settings_store, _| {
                let mut current_project_settings = ProjectSettings::default();
                let mut context_servers_map = HashMap::new();
                context_servers_map.insert(server_id_str.into(), project_config.clone());
                current_project_settings.context_servers = context_servers_map;

                settings_store
                    .set_project_settings(
                        WorktreeId::default(), // Use a dummy worktree ID
                        Path::new(""), // Dummy path
                        &current_project_settings,
                    )
                    .unwrap();
            });
            
            // Trigger the store to update its internal server list based on the new settings
            css.available_context_servers_changed(cx_model);
            css
        });
        
        // Wait for available_context_servers_changed to complete its async work
        cx.run_until_parked();


        TestSetup {
            project,
            store,
            server_id: ContextServerId(server_id_str.into()),
            cx: cx.clone(),
        }
    }


    #[gpui::test]
    async fn test_global_override_always_allow(mut cx: TestAppContext) {
        let server_id_str = "test_server_global_override";
        let setup = setup_environment(&mut cx, server_id_str, None).await;
        let mcp_tool_def = mcp_tool("any_tool");
        let tool = ContextServerTool::new(setup.store.clone(), setup.server_id.clone(), mcp_tool_def);

        cx.update(|c| {
            AssistantSettings::override_global(
                AssistantSettings {
                    always_allow_tool_actions: true,
                    ..Default::default()
                },
                c,
            );
        });

        let needs_confirmation = cx.read(|c| tool.needs_confirmation(&serde_json::Value::Null, c));
        assert!(!needs_confirmation, "Should not need confirmation when global override is true");
    }

    #[gpui::test]
    async fn test_no_specific_config_defaults_to_true(mut cx: TestAppContext) {
        let server_id_str = "test_server_no_config";
         // Setup with confirmation_settings = None by ensuring it's not in project_settings
        cx.update_global::<SettingsStore, _>(|settings_store, _| {
            let mut current_project_settings = ProjectSettings::default();
            current_project_settings.context_servers = HashMap::new(); // Ensure our server is not configured
            settings_store
                .set_project_settings(
                    WorktreeId::default(),
                    Path::new(""),
                    &current_project_settings,
                )
                .unwrap();
        });

        let setup = setup_environment(&mut cx, server_id_str, None).await; // Pass None initially
        
        // After setup_environment, ContextServerStore might have an entry if default project settings were used.
        // We need to ensure that for *this specific server_id*, get_confirmation_settings returns None.
        // The current setup_environment tries to add the server_id with `confirmation_settings`.
        // Let's adjust: `setup_environment` will add the server, but we'll ensure `zed_tool_confirmation` is None.
        // The test for "no specific config" means the *server* is configured, but *without* zed_tool_confirmation block.

        let mcp_tool_def = mcp_tool("any_tool");
        let tool = ContextServerTool::new(setup.store.clone(), setup.server_id.clone(), mcp_tool_def);

        cx.update(|c| {
            AssistantSettings::override_global(
                AssistantSettings {
                    always_allow_tool_actions: false,
                    ..Default::default()
                },
                c,
            );
        });
        
        // Confirm that get_confirmation_settings actually returns None for this server
        let settings_from_store = setup.store.read_with(&cx, |s, _| s.get_confirmation_settings(&setup.server_id));
        assert!(settings_from_store.is_none(), "Store should not have confirmation settings for this server ID to test this case.");


        let needs_confirmation = cx.read(|c| tool.needs_confirmation(&serde_json::Value::Null, c));
        assert!(needs_confirmation, "Should need confirmation by default when no specific config is found");
    }

    #[gpui::test]
    async fn test_server_default_confirmation(mut cx: TestAppContext) {
        let server_id_str = "test_server_default";
        let mcp_tool_def = mcp_tool("any_tool");

        // Case 1: default_needs_confirmation: Some(false)
        let settings_false = ZedToolConfirmationSettings {
            default_needs_confirmation: Some(false),
            tools: HashMap::new(),
        };
        let setup_false = setup_environment(&mut cx, server_id_str, Some(settings_false)).await;
        let tool_false = ContextServerTool::new(setup_false.store.clone(), setup_false.server_id.clone(), mcp_tool_def.clone());
        cx.update(|c| AssistantSettings::override_global(AssistantSettings { always_allow_tool_actions: false, ..Default::default() }, c));
        assert!(!cx.read(|c| tool_false.needs_confirmation(&serde_json::Value::Null, c)), "Should be false due to server default");

        // Case 2: default_needs_confirmation: Some(true)
        let settings_true = ZedToolConfirmationSettings {
            default_needs_confirmation: Some(true),
            tools: HashMap::new(),
        };
        let setup_true = setup_environment(&mut cx, server_id_str, Some(settings_true)).await;
        let tool_true = ContextServerTool::new(setup_true.store.clone(), setup_true.server_id.clone(), mcp_tool_def.clone());
        cx.update(|c| AssistantSettings::override_global(AssistantSettings { always_allow_tool_actions: false, ..Default::default() }, c));
        assert!(cx.read(|c| tool_true.needs_confirmation(&serde_json::Value::Null, c)), "Should be true due to server default");
        
        // Case 3: default_needs_confirmation: None
        let settings_none = ZedToolConfirmationSettings {
            default_needs_confirmation: None,
            tools: HashMap::new(),
        };
        let setup_none = setup_environment(&mut cx, server_id_str, Some(settings_none)).await;
        let tool_none = ContextServerTool::new(setup_none.store.clone(), setup_none.server_id.clone(), mcp_tool_def.clone());
        cx.update(|c| AssistantSettings::override_global(AssistantSettings { always_allow_tool_actions: false, ..Default::default() }, c));
        assert!(cx.read(|c| tool_none.needs_confirmation(&serde_json::Value::Null, c)), "Should default to true when server default is None");
    }

    #[gpui::test]
    async fn test_tool_specific_override(mut cx: TestAppContext) {
        let server_id_str = "test_server_specific";
        let specific_tool_name = "my_tool_name";
        let other_tool_name = "another_tool_name";

        cx.update(|c| AssistantSettings::override_global(AssistantSettings { always_allow_tool_actions: false, ..Default::default() }, c));

        // Scenario 1: Server default true, specific tool false
        let mut tools_map1 = HashMap::new();
        tools_map1.insert(specific_tool_name.to_string(), false);
        let settings1 = ZedToolConfirmationSettings {
            default_needs_confirmation: Some(true),
            tools: tools_map1,
        };
        let setup1 = setup_environment(&mut cx, server_id_str, Some(settings1)).await;

        let specific_mcp_tool1 = mcp_tool(specific_tool_name);
        let tool_specific1 = ContextServerTool::new(setup1.store.clone(), setup1.server_id.clone(), specific_mcp_tool1);
        assert!(!cx.read(|c| tool_specific1.needs_confirmation(&serde_json::Value::Null, c)), "Specific tool override to false failed");
        
        let other_mcp_tool1 = mcp_tool(other_tool_name);
        let tool_other1 = ContextServerTool::new(setup1.store.clone(), setup1.server_id.clone(), other_mcp_tool1);
        assert!(cx.read(|c| tool_other1.needs_confirmation(&serde_json::Value::Null, c)), "Fallback to server default true failed for other tool");

        // Scenario 2: Server default false, specific tool true
        let mut tools_map2 = HashMap::new();
        tools_map2.insert(specific_tool_name.to_string(), true);
        let settings2 = ZedToolConfirmationSettings {
            default_needs_confirmation: Some(false),
            tools: tools_map2,
        };
        let setup2 = setup_environment(&mut cx, server_id_str, Some(settings2)).await;
        
        let specific_mcp_tool2 = mcp_tool(specific_tool_name);
        let tool_specific2 = ContextServerTool::new(setup2.store.clone(), setup2.server_id.clone(), specific_mcp_tool2);
        assert!(cx.read(|c| tool_specific2.needs_confirmation(&serde_json::Value::Null, c)), "Specific tool override to true failed");

        let other_mcp_tool2 = mcp_tool(other_tool_name);
        let tool_other2 = ContextServerTool::new(setup2.store.clone(), setup2.server_id.clone(), other_mcp_tool2);
        assert!(!cx.read(|c| tool_other2.needs_confirmation(&serde_json::Value::Null, c)), "Fallback to server default false failed for other tool");
    }
}

impl Tool for ContextServerTool {
    fn name(&self) -> String {
        self.tool.name.clone()
    }

    fn description(&self) -> String {
        self.tool.description.clone().unwrap_or_default()
    }

    fn icon(&self) -> IconName {
        IconName::Cog
    }

    fn source(&self) -> ToolSource {
        ToolSource::ContextServer {
            id: self.server_id.clone().0.into(),
        }
    }

    fn needs_confirmation(&self, _input: &serde_json::Value, cx: &App) -> bool {
        // 1. Check global override from AssistantSettings
        if AssistantSettings::get_global(cx).always_allow_tool_actions {
            return false;
        }

        // 2. Access confirmation settings from ContextServerStore
        let confirmation_settings_opt = self
            .store
            .read(cx)
            .get_confirmation_settings(&self.server_id);

        if let Some(confirmation_settings) = confirmation_settings_opt {
            // Check specific tool override
            if let Some(specific_confirmation) = confirmation_settings.tools.get(&self.tool.name) {
                return *specific_confirmation;
            }
            // Check server default, defaulting to true if None
            return confirmation_settings
                .default_needs_confirmation
                .unwrap_or(true);
        }

        // 3. Default to true if no specific configuration for this server is found
        true
    }

    fn input_schema(&self, format: LanguageModelToolSchemaFormat) -> Result<serde_json::Value> {
        let mut schema = self.tool.input_schema.clone();
        assistant_tool::adapt_schema_to_format(&mut schema, format)?;
        Ok(match schema {
            serde_json::Value::Null => {
                serde_json::json!({ "type": "object", "properties": [] })
            }
            serde_json::Value::Object(map) if map.is_empty() => {
                serde_json::json!({ "type": "object", "properties": [] })
            }
            _ => schema,
        })
    }

    fn ui_text(&self, _input: &serde_json::Value) -> String {
        format!("Run MCP tool `{}`", self.tool.name)
    }

    fn run(
        self: Arc<Self>,
        input: serde_json::Value,
        _request: Arc<LanguageModelRequest>,
        _project: Entity<Project>,
        _action_log: Entity<ActionLog>,
        _model: Arc<dyn LanguageModel>,
        _window: Option<AnyWindowHandle>,
        cx: &mut App,
    ) -> ToolResult {
        if let Some(server) = self.store.read(cx).get_running_server(&self.server_id) {
            let tool_name = self.tool.name.clone();
            let server_clone = server.clone();
            let input_clone = input.clone();

            cx.spawn(async move |_cx| {
                let Some(protocol) = server_clone.client() else {
                    bail!("Context server not initialized");
                };

                let arguments = if let serde_json::Value::Object(map) = input_clone {
                    Some(map.into_iter().collect())
                } else {
                    None
                };

                log::trace!(
                    "Running tool: {} with arguments: {:?}",
                    tool_name,
                    arguments
                );
                let response = protocol.run_tool(tool_name, arguments).await?;

                let mut captured_image: Option<LanguageModelImage> = None;
                let mut text_parts: Vec<String> = Vec::new();

                for content_part in response.content {
                    match content_part {
                        types::ToolResponseContent::Text { text } => {
                            text_parts.push(text);
                        }
                        types::ToolResponseContent::Image { data, mime_type } => {
                            if mime_type == "image/png" {
                                if captured_image.is_none() {
                                    captured_image = Some(LanguageModelImage {
                                        source: data.into(),
                                        size: Size::default(),
                                    });
                                } else {
                                    log::warn!("Multiple images in tool response, only processing the first one.");
                                }
                            } else {
                                log::warn!("MCP tool returned non-PNG image ({}). Representing as text.", mime_type);
                                text_parts.push(format!("Tool returned an image of type {} (content not displayed in this view)", mime_type));
                            }
                        }
                        types::ToolResponseContent::Resource { .. } => {
                            log::warn!("Ignoring resource content from tool response as it's not supported.");
                        }
                    }
                }

                let intermediate_lm_content = if let Some(image) = captured_image {
                    LmToolResultContent::Image(image)
                } else {
                    LmToolResultContent::Text(text_parts.join("\n").into())
                };

                let final_assistant_tool_content = match intermediate_lm_content {
                    LmToolResultContent::Text(s) => {
                        AssistantToolResultContent::Text(s.to_string())
                    }
                    LmToolResultContent::Image(img) => {
                        AssistantToolResultContent::Image(img)
                    }
                    LmToolResultContent::WrappedText(wt) => {
                        AssistantToolResultContent::Text(wt.text.to_string())
                    }
                };

                Ok(ToolResultOutput { content: final_assistant_tool_content, output: None })
            })
            .into()
        } else {
            Task::ready(Err(anyhow!("Context server not found"))).into()
        }
    }
}
