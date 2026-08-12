/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */

//! Integration tests for the IntegrationNode pipeline.

use r8r::integrations::loader::IntegrationLoader;
use r8r::integrations::node::build_http_config;
use r8r::integrations::validator::validate_and_resolve_params;
use serde_json::json;

#[test]
fn test_full_pipeline_github_create_issue() {
    let loader = IntegrationLoader::new();
    let def = loader.get("github").unwrap();
    let op = def.operations.get("create_issue").unwrap();
    let params = json!({
        "owner": "octocat",
        "repo": "hello-world",
        "title": "Test issue",
        "labels": ["bug", "help wanted"]
    });
    let resolved = validate_and_resolve_params(&op.params, &params).unwrap();
    let config = build_http_config(&def, op, &resolved, Some("my_token"));
    assert_eq!(
        config["url"],
        "https://api.github.com/repos/octocat/hello-world/issues"
    );
    assert_eq!(config["method"], "POST");
    assert_eq!(config["body"]["title"], "Test issue");
    assert!(config["body"]["labels"].is_array());
    assert_eq!(config["body"]["labels"][0], "bug");
    assert_eq!(config["credential"], "my_token");
    assert_eq!(config["headers"]["Accept"], "application/vnd.github+json");
}

#[test]
fn test_full_pipeline_openai_completion() {
    let loader = IntegrationLoader::new();
    let def = loader.get("openai").unwrap();
    let op = def.operations.get("chat_completion").unwrap();
    let params = json!({
        "model": "gpt-4",
        "messages": [{"role": "user", "content": "Hello"}],
        "temperature": 0.7
    });
    let resolved = validate_and_resolve_params(&op.params, &params).unwrap();
    let config = build_http_config(&def, op, &resolved, Some("openai_key"));
    assert_eq!(config["url"], "https://api.openai.com/v1/chat/completions");
    assert_eq!(config["body"]["model"], "gpt-4");
    assert!(config["body"]["messages"].is_array());
    assert_eq!(config["body"]["temperature"], 0.7);
}

#[test]
fn test_full_pipeline_slack_send_message() {
    let loader = IntegrationLoader::new();
    let def = loader.get("slack").unwrap();
    let op = def.operations.get("send_message").unwrap();
    let params = json!({"channel": "#general", "text": "Hello from r8r!"});
    let resolved = validate_and_resolve_params(&op.params, &params).unwrap();
    let config = build_http_config(&def, op, &resolved, Some("slack_token"));
    assert_eq!(config["url"], "https://slack.com/api/chat.postMessage");
    assert_eq!(config["body"]["channel"], "#general");
    assert_eq!(config["body"]["text"], "Hello from r8r!");
}

#[test]
fn test_all_builtin_definitions_parse() {
    let loader = IntegrationLoader::new();
    let services = loader.list_all();
    assert!(services.contains(&"github".to_string()));
    assert!(services.contains(&"slack".to_string()));
    assert!(services.contains(&"openai".to_string()));
    assert!(services.contains(&"stripe".to_string()));
    assert!(services.contains(&"notion".to_string()));
    assert!(services.contains(&"telegram".to_string()));
    assert!(services.contains(&"whatsapp".to_string()));
    assert!(services.contains(&"google-sheets".to_string()));
    assert!(services.contains(&"airtable".to_string()));
    assert!(services.contains(&"discord".to_string()));
    assert!(services.contains(&"resend".to_string()));
    assert!(services.contains(&"twilio".to_string()));
    assert!(services.contains(&"shopify".to_string()));
    assert!(services.contains(&"hubspot".to_string()));
    assert!(services.contains(&"x".to_string()));
    assert!(services.contains(&"linkedin".to_string()));
    assert!(services.contains(&"youtube".to_string()));
    assert!(services.contains(&"google-drive".to_string()));
    assert!(services.contains(&"google-calendar".to_string()));
    assert!(services.contains(&"typeform".to_string()));
    assert!(services.contains(&"cal.com".to_string()));
    assert!(services.contains(&"supabase".to_string()));
    assert!(services.contains(&"exa".to_string()));
    assert!(services.contains(&"coingecko".to_string()));
    assert!(services.contains(&"elevenlabs".to_string()));
    assert!(services.contains(&"pdl".to_string()));
    assert!(services.contains(&"apollo".to_string()));
    assert!(services.contains(&"defillama".to_string()));
    assert!(services.contains(&"stablecoins".to_string()));
    assert!(services.contains(&"yields".to_string()));
    assert!(services.contains(&"browserbase".to_string()));
    assert!(services.contains(&"apify".to_string()));
    for service in &services {
        let def = loader.get(service).unwrap();
        assert!(
            !def.operations.is_empty(),
            "Service '{}' has no operations",
            service
        );
        assert!(
            !def.base_url.is_empty(),
            "Service '{}' has no base_url",
            service
        );
    }
}

#[test]
fn test_full_pipeline_telegram_send_message() {
    let loader = IntegrationLoader::new();
    let def = loader.get("telegram").unwrap();
    let op = def.operations.get("send_message").unwrap();
    let params = json!({"chat_id": "@my_channel", "text": "Hello from r8r!"});
    let resolved = validate_and_resolve_params(&op.params, &params).unwrap();
    let config = build_http_config(&def, op, &resolved, Some("123456:ABC-DEF"));
    // Bot token must be embedded in the URL path, not a header
    assert_eq!(
        config["url"],
        "https://api.telegram.org/bot123456:ABC-DEF/sendMessage"
    );
    assert_eq!(config["method"], "POST");
    assert_eq!(config["body"]["chat_id"], "@my_channel");
    assert_eq!(config["body"]["text"], "Hello from r8r!");
    assert_eq!(config["credential"], "123456:ABC-DEF");
}

#[test]
fn test_full_pipeline_whatsapp_send_text() {
    let loader = IntegrationLoader::new();
    let def = loader.get("whatsapp").unwrap();
    let op = def.operations.get("send_text").unwrap();
    let params = json!({
        "phone_number_id": "123456789",
        "to": "60123456789",
        "text": "Hello from r8r!"
    });
    let resolved = validate_and_resolve_params(&op.params, &params).unwrap();
    let config = build_http_config(&def, op, &resolved, Some("waba_token"));
    assert_eq!(
        config["url"],
        "https://graph.facebook.com/v21.0/123456789/messages"
    );
    assert_eq!(config["body"]["messaging_product"], "whatsapp");
    assert_eq!(config["body"]["to"], "60123456789");
    assert_eq!(config["body"]["type"], "text");
    assert_eq!(config["body"]["text"]["body"], "Hello from r8r!");
}

#[test]
fn test_full_pipeline_google_sheets_append() {
    let loader = IntegrationLoader::new();
    let def = loader.get("google-sheets").unwrap();
    let op = def.operations.get("append_values").unwrap();
    let params = json!({
        "spreadsheet_id": "abc123",
        "range": "Sheet1!A1",
        "values": [["Lead", "Kimi", "0123456789"]],
        "value_input_option": "RAW"
    });
    let resolved = validate_and_resolve_params(&op.params, &params).unwrap();
    let config = build_http_config(&def, op, &resolved, Some("g_token"));
    let url = config["url"].as_str().unwrap();
    assert!(url.starts_with(
        "https://sheets.googleapis.com/v4/spreadsheets/abc123/values/Sheet1!A1:append?"
    ));
    // Query param order is non-deterministic (HashMap) — check both present
    assert!(url.contains("insertDataOption=INSERT_ROWS"));
    assert!(url.contains("valueInputOption=RAW"));
    assert_eq!(config["method"], "POST");
    assert!(config["body"]["values"].is_array());
    assert_eq!(config["body"]["values"][0][0], "Lead");
}

#[test]
fn test_full_pipeline_airtable_create_records() {
    let loader = IntegrationLoader::new();
    let def = loader.get("airtable").unwrap();
    let op = def.operations.get("create_records").unwrap();
    let params = json!({
        "base_id": "appABC",
        "table_name": "Leads",
        "records": [{"fields": {"Name": "Test Lead", "Status": "New"}}]
    });
    let resolved = validate_and_resolve_params(&op.params, &params).unwrap();
    let config = build_http_config(&def, op, &resolved, Some("pat_token"));
    assert_eq!(config["url"], "https://api.airtable.com/v0/appABC/Leads");
    assert_eq!(config["method"], "POST");
    assert!(config["body"]["records"].is_array());
    assert_eq!(config["body"]["records"][0]["fields"]["Name"], "Test Lead");
}

#[test]
fn test_full_pipeline_discord_send_message() {
    let loader = IntegrationLoader::new();
    let def = loader.get("discord").unwrap();
    let op = def.operations.get("send_message").unwrap();
    let params = json!({"channel_id": "123456789", "content": "Hello from r8r!"});
    let resolved = validate_and_resolve_params(&op.params, &params).unwrap();
    let config = build_http_config(&def, op, &resolved, Some("Bot MTIz"));
    assert_eq!(
        config["url"],
        "https://discord.com/api/v10/channels/123456789/messages"
    );
    assert_eq!(config["method"], "POST");
    assert_eq!(config["body"]["content"], "Hello from r8r!");
    assert_eq!(config["auth_type"], "header:Authorization");
    assert_eq!(config["credential"], "Bot MTIz");
}

#[test]
fn test_user_override_takes_precedence() {
    let loader = IntegrationLoader::new();
    let github = loader.get_builtin("github");
    assert!(github.is_some());
    assert_eq!(github.unwrap().base_url, "https://api.github.com");
}

#[test]
fn test_full_pipeline_resend_send_email() {
    let loader = IntegrationLoader::new();
    let def = loader.get("resend").unwrap();
    let op = def.operations.get("send_email").unwrap();
    let params = json!({
        "from": "onboarding@kitakod.com",
        "to": ["lead@example.com"],
        "subject": "Halo",
        "html": "<p>Test</p>"
    });
    let resolved = validate_and_resolve_params(&op.params, &params).unwrap();
    let config = build_http_config(&def, op, &resolved, Some("re_key"));
    assert_eq!(config["url"], "https://api.resend.com/emails");
    assert_eq!(config["method"], "POST");
    assert_eq!(config["body"]["from"], "onboarding@kitakod.com");
    assert_eq!(config["body"]["subject"], "Halo");
    assert_eq!(config["auth_type"], "bearer");
    assert_eq!(config["credential"], "re_key");
}

#[test]
fn test_full_pipeline_twilio_send_sms_form_body() {
    let loader = IntegrationLoader::new();
    let def = loader.get("twilio").unwrap();
    let op = def.operations.get("send_sms").unwrap();
    let params = json!({
        "account_sid": "AC123",
        "to": "+60123456789",
        "from": "+15005550006",
        "body": "Hello from r8r!"
    });
    let resolved = validate_and_resolve_params(&op.params, &params).unwrap();
    let config = build_http_config(&def, op, &resolved, Some("AC123:auth_token"));
    assert_eq!(
        config["url"],
        "https://api.twilio.com/2010-04-01/Accounts/AC123/Messages.json"
    );
    assert_eq!(config["method"], "POST");
    assert_eq!(config["body_type"], "form");
    assert_eq!(config["body"]["To"], "+60123456789");
    assert_eq!(config["body"]["Body"], "Hello from r8r!");
    assert_eq!(config["auth_type"], "basic");
}

#[test]
fn test_full_pipeline_shopify_dynamic_base_url() {
    let loader = IntegrationLoader::new();
    let def = loader.get("shopify").unwrap();
    let op = def.operations.get("list_products").unwrap();
    let params = json!({"shop": "my-store"});
    let resolved = validate_and_resolve_params(&op.params, &params).unwrap();
    let config = build_http_config(&def, op, &resolved, Some("shpat_token"));
    // base_url is templated with the shop subdomain (plus default query limit)
    let url = config["url"].as_str().unwrap();
    assert!(url.starts_with("https://my-store.myshopify.com/admin/api/2024-01/products.json"));
    assert!(url.contains("limit=50"));
    assert_eq!(config["method"], "GET");
    assert_eq!(config["auth_type"], "header:X-Shopify-Access-Token");
    assert_eq!(config["credential"], "shpat_token");
}

#[test]
fn test_full_pipeline_hubspot_create_contact() {
    let loader = IntegrationLoader::new();
    let def = loader.get("hubspot").unwrap();
    let op = def.operations.get("create_contact").unwrap();
    let params = json!({
        "email": "lead@example.com",
        "firstname": "Ahmad",
        "lastname": "Bin Ali",
        "phone": "+60123456789"
    });
    let resolved = validate_and_resolve_params(&op.params, &params).unwrap();
    let config = build_http_config(&def, op, &resolved, Some("pat_key"));
    assert_eq!(
        config["url"],
        "https://api.hubapi.com/crm/v3/objects/contacts"
    );
    assert_eq!(config["method"], "POST");
    assert_eq!(config["body"]["properties"]["email"], "lead@example.com");
    assert_eq!(config["body"]["properties"]["firstname"], "Ahmad");
    assert_eq!(config["auth_type"], "bearer");
}

#[test]
fn test_full_pipeline_x_create_tweet() {
    let loader = IntegrationLoader::new();
    let def = loader.get("x").unwrap();
    let op = def.operations.get("create_tweet").unwrap();
    let params = json!({"text": "Hello from r8r! 🚀"});
    let resolved = validate_and_resolve_params(&op.params, &params).unwrap();
    let config = build_http_config(&def, op, &resolved, Some("x_bearer"));
    assert_eq!(config["url"], "https://api.x.com/2/tweets");
    assert_eq!(config["method"], "POST");
    assert_eq!(config["body"]["text"], "Hello from r8r! 🚀");
    assert_eq!(config["auth_type"], "bearer");
}

#[test]
fn test_full_pipeline_linkedin_get_me() {
    let loader = IntegrationLoader::new();
    let def = loader.get("linkedin").unwrap();
    let op = def.operations.get("get_me").unwrap();
    let params = json!({});
    let resolved = validate_and_resolve_params(&op.params, &params).unwrap();
    let config = build_http_config(&def, op, &resolved, Some("li_token"));
    assert_eq!(config["url"], "https://api.linkedin.com/v2/userinfo");
    assert_eq!(config["method"], "GET");
    assert_eq!(config["auth_type"], "bearer");
    assert_eq!(config["headers"]["X-Restli-Protocol-Version"], "2.0.0");
}

#[test]
fn test_full_pipeline_youtube_credential_in_query() {
    let loader = IntegrationLoader::new();
    let def = loader.get("youtube").unwrap();
    let op = def.operations.get("search_videos").unwrap();
    let params = json!({"query": "halal certification", "max_results": 5});
    let resolved = validate_and_resolve_params(&op.params, &params).unwrap();
    let config = build_http_config(&def, op, &resolved, Some("yt_api_key"));
    let url = config["url"].as_str().unwrap();
    assert!(url.starts_with("https://www.googleapis.com/youtube/v3/search?"));
    // API key injected as `key` query param (underscores are not percent-encoded)
    assert!(url.contains("part=snippet"));
    assert!(url.contains("key=yt_api_key"));
    assert!(url.contains("q=halal%20certification"));
    assert_eq!(config["method"], "GET");
}

#[test]
fn test_full_pipeline_google_drive_list_files() {
    let loader = IntegrationLoader::new();
    let def = loader.get("google-drive").unwrap();
    let op = def.operations.get("list_files").unwrap();
    let params = json!({"query": "name contains 'report'", "page_size": 10});
    let resolved = validate_and_resolve_params(&op.params, &params).unwrap();
    let config = build_http_config(&def, op, &resolved, Some("g_drive_token"));
    let url = config["url"].as_str().unwrap();
    assert!(url.starts_with("https://www.googleapis.com/drive/v3/files?"));
    assert!(url.contains("q=name%20contains%20%27report%27"));
    assert_eq!(config["method"], "GET");
    assert_eq!(config["auth_type"], "bearer");
}

#[test]
fn test_full_pipeline_google_calendar_create_event() {
    let loader = IntegrationLoader::new();
    let def = loader.get("google-calendar").unwrap();
    let op = def.operations.get("create_event").unwrap();
    let params = json!({
        "calendar_id": "primary",
        "summary": "Meeting",
        "start": "2026-08-20T10:00:00+08:00",
        "end": "2026-08-20T11:00:00+08:00"
    });
    let resolved = validate_and_resolve_params(&op.params, &params).unwrap();
    let config = build_http_config(&def, op, &resolved, Some("g_cal_token"));
    assert_eq!(
        config["url"],
        "https://www.googleapis.com/calendar/v3/calendars/primary/events"
    );
    assert_eq!(config["method"], "POST");
    assert_eq!(config["body"]["summary"], "Meeting");
    assert_eq!(
        config["body"]["start"]["dateTime"],
        "2026-08-20T10:00:00+08:00"
    );
    assert_eq!(
        config["body"]["end"]["dateTime"],
        "2026-08-20T11:00:00+08:00"
    );
    assert_eq!(config["auth_type"], "bearer");
}

#[test]
fn test_full_pipeline_typeform_list_responses() {
    let loader = IntegrationLoader::new();
    let def = loader.get("typeform").unwrap();
    let op = def.operations.get("list_responses").unwrap();
    let params = json!({"form_id": "abc123", "page_size": 25});
    let resolved = validate_and_resolve_params(&op.params, &params).unwrap();
    let config = build_http_config(&def, op, &resolved, Some("tf_token"));
    let url = config["url"].as_str().unwrap();
    assert!(url.starts_with("https://api.typeform.com/forms/abc123/responses?"));
    assert!(url.contains("page_size=25"));
    assert_eq!(config["method"], "GET");
    assert_eq!(config["auth_type"], "bearer");
}

#[test]
fn test_full_pipeline_calcom_credential_in_query() {
    let loader = IntegrationLoader::new();
    let def = loader.get("cal.com").unwrap();
    let op = def.operations.get("list_event_types").unwrap();
    let params = json!({});
    let resolved = validate_and_resolve_params(&op.params, &params).unwrap();
    let config = build_http_config(&def, op, &resolved, Some("cal_key"));
    let url = config["url"].as_str().unwrap();
    assert!(url.starts_with("https://api.cal.com/v1/event-types?"));
    assert!(url.contains("apiKey=cal_key"));
    assert_eq!(config["method"], "GET");
}

#[test]
fn test_full_pipeline_supabase_dual_headers() {
    let loader = IntegrationLoader::new();
    let def = loader.get("supabase").unwrap();
    let op = def.operations.get("insert").unwrap();
    let params = json!({
        "project_ref": "abcd1234",
        "table": "leads",
        "rows": [{"name": "Ahmad", "status": "new"}]
    });
    let resolved = validate_and_resolve_params(&op.params, &params).unwrap();
    let config = build_http_config(&def, op, &resolved, Some("service_role_key"));
    assert_eq!(config["url"], "https://abcd1234.supabase.co/rest/v1/leads");
    assert_eq!(config["method"], "POST");
    // Service-level header injects credential as apikey
    assert_eq!(config["headers"]["apikey"], "service_role_key");
    assert_eq!(config["auth_type"], "header:Authorization");
    assert!(config["body"].is_array());
    assert_eq!(config["body"][0]["name"], "Ahmad");
}

#[test]
fn test_full_pipeline_exa_x_api_key_header() {
    let loader = IntegrationLoader::new();
    let def = loader.get("exa").unwrap();
    let op = def.operations.get("search").unwrap();
    let params = json!({"query": "halal certification malaysia", "num_results": 3});
    let resolved = validate_and_resolve_params(&op.params, &params).unwrap();
    let config = build_http_config(&def, op, &resolved, Some("exa_key"));
    assert_eq!(config["url"], "https://api.exa.ai/search");
    assert_eq!(config["method"], "POST");
    // Exa uses x-api-key header (NOT Bearer)
    assert_eq!(config["headers"]["x-api-key"], "exa_key");
    assert_eq!(config["body"]["query"], "halal certification malaysia");
}

#[test]
fn test_full_pipeline_coingecko_public_no_auth() {
    let loader = IntegrationLoader::new();
    let def = loader.get("coingecko").unwrap();
    let op = def.operations.get("get_price").unwrap();
    let params = json!({"ids": "bitcoin,ethereum", "vs_currencies": "usd,myr"});
    let resolved = validate_and_resolve_params(&op.params, &params).unwrap();
    let config = build_http_config(&def, op, &resolved, None);
    let url = config["url"].as_str().unwrap();
    assert!(url.starts_with("https://api.coingecko.com/api/v3/simple/price?"));
    assert!(url.contains("ids=bitcoin") && url.contains("vs_currencies=usd"));
    assert_eq!(config["method"], "GET");
}

#[test]
fn test_full_pipeline_defillama_public_no_auth() {
    let loader = IntegrationLoader::new();
    let def = loader.get("defillama").unwrap();
    let op = def.operations.get("get_protocols").unwrap();
    let params = json!({});
    let resolved = validate_and_resolve_params(&op.params, &params).unwrap();
    let config = build_http_config(&def, op, &resolved, None);
    assert_eq!(config["url"], "https://api.llama.fi/protocols");
    assert_eq!(config["method"], "GET");
}

#[test]
fn test_full_pipeline_stablecoins_yields_subdomains() {
    let loader = IntegrationLoader::new();
    let def = loader.get("stablecoins").unwrap();
    let op = def.operations.get("get_stablecoins").unwrap();
    let resolved = validate_and_resolve_params(&op.params, &json!({})).unwrap();
    let config = build_http_config(&def, op, &resolved, None);
    let url = config["url"].as_str().unwrap();
    assert!(url.starts_with("https://stablecoins.llama.fi/stablecoins"));

    let def = loader.get("yields").unwrap();
    let op = def.operations.get("get_pools").unwrap();
    let resolved = validate_and_resolve_params(&op.params, &json!({})).unwrap();
    let config = build_http_config(&def, op, &resolved, None);
    let url = config["url"].as_str().unwrap();
    assert!(url.starts_with("https://yields.llama.fi/pools"));
}

#[test]
fn test_full_pipeline_apify_raw_body_and_bearer() {
    let loader = IntegrationLoader::new();
    let def = loader.get("apify").unwrap();
    let op = def.operations.get("run_actor").unwrap();
    let params = json!({
        "actor_id": "compass~google-maps-extractor",
        "input": {"searchStrings": ["kuala lumpur"], "maxCrawledPlacesPerSearch": 5}
    });
    let resolved = validate_and_resolve_params(&op.params, &params).unwrap();
    let config = build_http_config(&def, op, &resolved, Some("apify_token"));
    assert_eq!(
        config["url"],
        "https://api.apify.com/v2/actors/compass~google-maps-extractor/runs"
    );
    assert_eq!(config["method"], "POST");
    // Raw actor input is the whole body — no wrapper
    assert_eq!(config["body"]["searchStrings"][0], "kuala lumpur");
    assert_eq!(config["auth_type"], "bearer");
}

#[test]
fn test_full_pipeline_browserbase_bb_api_key_header() {
    let loader = IntegrationLoader::new();
    let def = loader.get("browserbase").unwrap();
    let op = def.operations.get("create_session").unwrap();
    let params = json!({});
    let resolved = validate_and_resolve_params(&op.params, &params).unwrap();
    let config = build_http_config(&def, op, &resolved, Some("bb_key"));
    assert_eq!(config["url"], "https://api.browserbase.com/v1/sessions");
    assert_eq!(config["method"], "POST");
    assert_eq!(config["headers"]["X-BB-API-Key"], "bb_key");
}

#[test]
fn test_full_pipeline_elevenlabs_xi_api_key() {
    let loader = IntegrationLoader::new();
    let def = loader.get("elevenlabs").unwrap();
    let op = def.operations.get("list_voices").unwrap();
    let resolved = validate_and_resolve_params(&op.params, &json!({})).unwrap();
    let config = build_http_config(&def, op, &resolved, Some("xi_key"));
    assert_eq!(config["url"], "https://api.elevenlabs.io/v2/voices");
    assert_eq!(config["method"], "GET");
    assert_eq!(config["headers"]["xi-api-key"], "xi_key");
}

#[test]
fn test_full_pipeline_pdl_api_key_in_query() {
    let loader = IntegrationLoader::new();
    let def = loader.get("pdl").unwrap();
    let op = def.operations.get("person_enrich").unwrap();
    let params = json!({
        "email": "ceo@acme.com",
        "first_name": "John",
        "last_name": "Doe",
        "company": "Acme Inc",
        "title": "CEO"
    });
    let resolved = validate_and_resolve_params(&op.params, &params).unwrap();
    let config = build_http_config(&def, op, &resolved, Some("pdl_key"));
    let url = config["url"].as_str().unwrap();
    assert!(url.starts_with("https://api.peopledatalabs.com/v5/person/enrich?"));
    assert!(url.contains("api_key=pdl_key"));
    assert!(url.contains("job_title=CEO"));
    assert_eq!(config["method"], "GET");
}

#[test]
fn test_full_pipeline_apollo_header_and_org_names() {
    let loader = IntegrationLoader::new();
    let def = loader.get("apollo").unwrap();
    let op = def.operations.get("search_people").unwrap();
    let params = json!({
        "q_keywords": "head of marketing",
        "person_titles": ["CMO"],
        "organization_names": ["Acme Inc"]
    });
    let resolved = validate_and_resolve_params(&op.params, &params).unwrap();
    let config = build_http_config(&def, op, &resolved, Some("apollo_key"));
    assert_eq!(config["url"], "https://api.apollo.io/v1/people/search");
    assert_eq!(config["method"], "POST");
    assert_eq!(config["headers"]["X-Api-Key"], "apollo_key");
    assert_eq!(config["body"]["organization_names"][0], "Acme Inc");
    assert_eq!(config["body"]["q_keywords"], "head of marketing");
}
