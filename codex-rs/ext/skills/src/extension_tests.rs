use std::sync::Mutex;

use codex_extension_api::ExtensionMetrics;
use codex_otel::THREAD_SKILLS_DESCRIPTION_TRUNCATED_CHARS_METRIC;
use codex_otel::THREAD_SKILLS_ENABLED_TOTAL_METRIC;
use codex_otel::THREAD_SKILLS_KEPT_TOTAL_METRIC;
use codex_otel::THREAD_SKILLS_TRUNCATED_METRIC;
use pretty_assertions::assert_eq;

use super::*;

#[derive(Default)]
struct RecordingMetrics {
    samples: Mutex<Vec<(String, i64)>>,
}

impl ExtensionMetrics for RecordingMetrics {
    fn histogram_with_boundaries(
        &self,
        name: &str,
        value: i64,
        _boundaries: &[f64],
        tags: &[(&str, &str)],
    ) {
        self.histogram(name, value, tags);
    }

    fn counter(&self, name: &str, _inc: i64, _tags: &[(&str, &str)]) {
        panic!("unexpected counter: {name}");
    }

    fn histogram(&self, name: &str, value: i64, _tags: &[(&str, &str)]) {
        self.samples
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((name.to_string(), value));
    }
}

#[test]
fn empty_catalog_records_zero_metrics_without_a_fragment() {
    let metrics = RecordingMetrics::default();

    let rendered = render_catalog(
        Some(&metrics),
        CatalogSurface::ThreadContext,
        &SkillCatalog::default(),
        /*include_skills_usage_instructions*/ false,
        SkillCatalogRenderPolicy::ExtensionCompatible,
        SkillMetadataBudget::Characters(8_000),
    );

    assert!(rendered.fragment.is_none());
    assert_eq!(rendered.warning_message, None);

    assert_eq!(
        *metrics
            .samples
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        vec![
            (THREAD_SKILLS_ENABLED_TOTAL_METRIC.to_string(), 0),
            (THREAD_SKILLS_KEPT_TOTAL_METRIC.to_string(), 0),
            (THREAD_SKILLS_TRUNCATED_METRIC.to_string(), 0),
            (
                THREAD_SKILLS_DESCRIPTION_TRUNCATED_CHARS_METRIC.to_string(),
                0,
            ),
        ]
    );
}

struct StaticCustomSkillProvider {
    entries: Vec<SkillCatalogEntry>,
}

impl crate::provider::SkillProvider for StaticCustomSkillProvider {
    fn list(
        &self,
        _query: SkillListQuery,
    ) -> crate::provider::SkillProviderFuture<'_, SkillCatalog> {
        let mut catalog = SkillCatalog::default();
        catalog.entries = self.entries.clone();
        Box::pin(async move { Ok(catalog) })
    }

    fn read<'a>(
        &'a self,
        _request: SkillReadRequest<'a>,
    ) -> crate::provider::SkillProviderFuture<'a, SkillReadResult> {
        Box::pin(async { Err(crate::catalog::SkillProviderError::new("unused")) })
    }

    fn search(
        &self,
        _request: crate::provider::SkillSearchRequest,
    ) -> crate::provider::SkillProviderFuture<'_, crate::catalog::SkillSearchResult> {
        Box::pin(async { Ok(crate::catalog::SkillSearchResult::default()) })
    }
}

struct NoopEventSink;

impl ExtensionEventSink for NoopEventSink {
    fn emit(&self, _event: codex_protocol::protocol::Event) {}
    fn emit_warning(&self, _warning: ExtensionWarning) {}
}

#[tokio::test]
async fn contribute_thread_context_uses_model_info_context_window_for_metadata_budget() {
    use crate::catalog::SkillAuthority;
    use crate::catalog::SkillPackageId;
    use crate::catalog::SkillResourceId;
    use crate::sources::SkillProviderSource;
    use codex_models_manager::model_info::model_info_from_slug;

    let entries = (0..20)
        .map(|index| {
            let id = format!("custom/skill-{index:02}");
            let name = format!("skill-{index:02}");
            let description = format!("skill-{index:02} {}", "x".repeat(450));
            SkillCatalogEntry::new(
                SkillPackageId(id.clone()),
                SkillAuthority::new(SkillSourceKind::Custom("custom".to_string()), "custom"),
                name,
                description,
                SkillResourceId::new("SKILL.md"),
            )
            .with_display_path(format!("skill://{id}/SKILL.md"))
        })
        .collect::<Vec<_>>();

    let providers = SkillProviders::new().with_provider(SkillProviderSource::new(
        SkillSourceKind::Custom("custom".to_string()),
        "custom",
        Arc::new(StaticCustomSkillProvider { entries }),
    ));
    let mut builder = ExtensionRegistryBuilder::with_event_sink(Arc::new(NoopEventSink));
    install_with_providers(&mut builder, providers, |config: &SkillsExtensionConfig| {
        config.clone()
    });
    let registry = builder.build();

    let config = SkillsExtensionConfig {
        include_instructions: true,
        max_context_tokens: None,
        bundled_skills_enabled: false,
        cloud_skill_enabled: false,
        shadow_selection_enabled: false,
    };

    let session_store = ExtensionData::new("session");
    let small_thread_store = ExtensionData::new("thread-small");
    registry.thread_lifecycle_contributors()[0]
        .on_thread_start(ThreadStartInput {
            config: &config,
            session_source: &codex_protocol::protocol::SessionSource::Cli,
            persistent_thread_state_available: true,
            environments: &[],
            mcp_resource_client: None,
            extension_metrics: None,
            session_store: &session_store,
            thread_store: &small_thread_store,
        })
        .await;
    let mut small_model = model_info_from_slug("small-context-model");
    small_model.context_window = None;
    small_model.max_context_window = None;
    small_model.effective_context_window_percent = 100;
    small_thread_store.insert(small_model);

    let small_fragments = registry.context_contributors()[0]
        .contribute_thread_context(&session_store, &small_thread_store)
        .await;
    assert_eq!(small_fragments.len(), 1);
    assert!(!small_fragments[0].text().contains(&"x".repeat(450)));

    let large_thread_store = ExtensionData::new("thread-large");
    registry.thread_lifecycle_contributors()[0]
        .on_thread_start(ThreadStartInput {
            config: &config,
            session_source: &codex_protocol::protocol::SessionSource::Cli,
            persistent_thread_state_available: true,
            environments: &[],
            mcp_resource_client: None,
            extension_metrics: None,
            session_store: &session_store,
            thread_store: &large_thread_store,
        })
        .await;
    let mut large_model = model_info_from_slug("large-context-model");
    large_model.context_window = Some(400_000);
    large_model.effective_context_window_percent = 100;
    large_thread_store.insert(large_model);

    let large_fragments = registry.context_contributors()[0]
        .contribute_thread_context(&session_store, &large_thread_store)
        .await;
    assert_eq!(large_fragments.len(), 1);
    assert!(large_fragments[0].text().contains(&"x".repeat(450)));
}
