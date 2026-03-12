# r8r Use Cases

r8r is designed to run the same way everywhere. A workflow written for a cloud server deploys unchanged to a Raspberry Pi at the edge, and the same YAML connects cloud and edge in hybrid architectures. This document shows concrete, working examples across all three deployment contexts.

---

## Cloud Use Cases

### 1. AI-Powered GitHub Issue Triage

A webhook-triggered workflow that classifies incoming GitHub issues, labels them, assigns the right engineer, and notifies the team on Slack.

```yaml
name: github-issue-triage
description: Classify and route new GitHub issues using AI
version: 1

triggers:
  - type: webhook
    config:
      path: /github/issues
      method: POST
      header_filter:
        - header: X-GitHub-Event
          value: "issues"

nodes:
  # Extract issue details from the webhook payload
  - id: extract-issue
    type: transform
    config:
      expression: |
        let payload = input;
        if payload.action != "opened" {
          return ();
        }
        ({
          "number": payload.issue.number,
          "title": payload.issue.title,
          "body": payload.issue.body,
          "author": payload.issue.user.login,
          "repo": payload.repository.full_name,
          "url": payload.issue.html_url
        })

  # Use AI to classify the issue
  - id: classify-issue
    type: agent
    config:
      provider: openai
      model: gpt-4o
      credential: openai
      prompt: |
        Classify this GitHub issue.

        Title: {{ nodes.extract-issue.output.title }}
        Body: {{ nodes.extract-issue.output.body }}

        Return JSON with:
        - priority: critical, high, medium, or low
        - category: bug, feature, docs, performance, security, question
        - assignee: one of [alice, bob, carol] based on category
          (alice=bugs/security, bob=features/performance, carol=docs/questions)
        - summary: one-sentence summary
      response_format: json
      json_schema:
        type: object
        required: [priority, category, assignee, summary]
        properties:
          priority: { type: string, enum: [critical, high, medium, low] }
          category: { type: string, enum: [bug, feature, docs, performance, security, question] }
          assignee: { type: string, enum: [alice, bob, carol] }
          summary: { type: string }
    depends_on: [extract-issue]

  # Apply labels and assign via GitHub API
  - id: label-and-assign
    type: http
    config:
      url: "https://api.github.com/repos/{{ nodes.extract-issue.output.repo }}/issues/{{ nodes.extract-issue.output.number }}"
      method: PATCH
      headers:
        Accept: "application/vnd.github.v3+json"
      body:
        labels:
          - "priority:{{ nodes.classify-issue.output.priority }}"
          - "{{ nodes.classify-issue.output.category }}"
        assignees:
          - "{{ nodes.classify-issue.output.assignee }}"
      credential: github_token
      auth_type: bearer
    depends_on: [classify-issue]

  # Notify the team on Slack
  - id: notify-slack
    type: slack
    config:
      channel: "#engineering-triage"
      credential: slack_token
      blocks:
        - type: section
          text:
            type: mrkdwn
            text: |
              *New Issue Triaged*
              <{{ nodes.extract-issue.output.url }}|#{{ nodes.extract-issue.output.number }}>: {{ nodes.extract-issue.output.title }}
              *Priority:* {{ nodes.classify-issue.output.priority }} | *Category:* {{ nodes.classify-issue.output.category }}
              *Assigned to:* {{ nodes.classify-issue.output.assignee }}
              *AI Summary:* {{ nodes.classify-issue.output.summary }}
    depends_on: [label-and-assign]
```

---

### 2. Competitive Price Monitor

A daily cron workflow that fetches competitor pricing, detects meaningful changes with AI analysis, and alerts the team on Slack.

```yaml
name: competitive-price-monitor
description: Daily competitor price check with AI-powered change analysis
version: 1

triggers:
  - type: cron
    schedule: "0 8 * * *"  # Daily at 8 AM
    timezone: UTC

nodes:
  # Fetch competitor pricing from their public API / page
  - id: fetch-competitor-a
    type: http
    config:
      url: "{{ env.COMPETITOR_A_PRICING_URL }}"
      method: GET
      timeout_seconds: 15
    retry:
      max_attempts: 2
      delay_seconds: 10
      backoff: exponential

  - id: fetch-competitor-b
    type: http
    config:
      url: "{{ env.COMPETITOR_B_PRICING_URL }}"
      method: GET
      timeout_seconds: 15
    retry:
      max_attempts: 2
      delay_seconds: 10
      backoff: exponential

  # Load yesterday's prices from the database
  - id: load-previous-prices
    type: database
    config:
      db_type: sqlite
      connection_string: "./data/prices.db"
      query: >
        SELECT competitor, product, price, recorded_at
        FROM price_history
        WHERE recorded_at = (SELECT MAX(recorded_at) FROM price_history)
      operation: query

  # Use AI to analyze the price changes
  - id: analyze-changes
    type: agent
    config:
      provider: anthropic
      model: claude-sonnet-4-5-20250929
      credential: anthropic
      system: "You are a competitive intelligence analyst. Be concise and data-driven."
      prompt: |
        Compare current vs previous competitor pricing.

        Current prices:
        Competitor A: {{ nodes.fetch-competitor-a.output.body }}
        Competitor B: {{ nodes.fetch-competitor-b.output.body }}

        Previous prices:
        {{ nodes.load-previous-prices.output.rows }}

        Return JSON:
        - significant_changes: array of {competitor, product, old_price, new_price, pct_change}
          (only include changes > 5%)
        - alert: true if any change > 10% or a new product appeared
        - summary: 2-3 sentence analysis
      response_format: json
      max_tokens: 1024
    depends_on: [fetch-competitor-a, fetch-competitor-b, load-previous-prices]

  # Store today's prices
  - id: store-prices
    type: database
    config:
      db_type: sqlite
      connection_string: "./data/prices.db"
      query: >
        INSERT INTO price_history (competitor, product, price, recorded_at)
        VALUES (?, ?, ?, datetime('now'))
      params:
        - "{{ nodes.fetch-competitor-a.output.body }}"
        - "all"
        - "0"
      operation: execute
    depends_on: [analyze-changes]

  # Send Slack alert if significant changes detected
  - id: check-alert
    type: if
    config:
      condition: "nodes.get(\"analyze-changes\").output.alert == true"
      true_node: send-alert
      false_node: log-quiet
    depends_on: [analyze-changes]

  - id: send-alert
    type: slack
    config:
      channel: "#competitive-intel"
      credential: slack_token
      text: |
        :rotating_light: *Competitor Price Alert*

        {{ nodes.analyze-changes.output.summary }}

        *Significant Changes:*
        {{ nodes.analyze-changes.output.significant_changes }}
    depends_on: [check-alert]

  - id: log-quiet
    type: debug
    config:
      label: "No significant price changes detected"
      level: info
    depends_on: [check-alert]

settings:
  timeout_seconds: 120
```

---

### 3. Content Pipeline with Human Approval

A workflow that receives draft content, runs AI quality review, and routes flagged content through human approval before publishing.

```yaml
name: content-pipeline
description: AI-reviewed content publishing with human approval gate
version: 1

triggers:
  - type: webhook
    config:
      path: /content/submit
      method: POST
      response_mode: full

nodes:
  # Extract and validate the submitted content
  - id: validate-input
    type: transform
    config:
      expression: |
        let draft = input;
        ({
          "title": draft.title,
          "body": draft.body,
          "author": draft.author,
          "publish_url": draft.publish_url
        })

  # AI review for quality and compliance
  - id: ai-review
    type: agent
    config:
      provider: openai
      model: gpt-4o
      credential: openai
      system: "You are a content quality and compliance reviewer."
      prompt: |
        Review this content draft for quality and compliance issues.

        Title: {{ nodes.validate-input.output.title }}
        Body: {{ nodes.validate-input.output.body }}

        Check for:
        1. Factual claims that need citations
        2. Potentially offensive or inappropriate language
        3. Brand guideline violations
        4. Grammar and clarity issues

        Return JSON:
        - score: 1-10 quality score
        - flagged: true if any compliance issue found
        - issues: array of {type, description, severity}
        - suggestion: brief improvement recommendation
      response_format: json
      json_schema:
        type: object
        required: [score, flagged, issues, suggestion]
        properties:
          score: { type: number }
          flagged: { type: boolean }
          issues: { type: array }
          suggestion: { type: string }
    depends_on: [validate-input]

  # Route based on AI review result
  - id: route-decision
    type: if
    config:
      condition: "nodes.get(\"ai-review\").output.flagged == true"
      true_node: human-review
      false_node: auto-publish
    depends_on: [ai-review]

  # Human approval gate for flagged content
  - id: human-review
    type: approval
    config:
      title: "Content flagged for review: {{ nodes.validate-input.output.title }}"
      description: |
        AI Score: {{ nodes.ai-review.output.score }}/10
        Issues: {{ nodes.ai-review.output.issues }}
        Suggestion: {{ nodes.ai-review.output.suggestion }}
      timeout_seconds: 86400       # 24-hour review window
      default_action: reject        # Auto-reject if no response
    depends_on: [route-decision]

  # Route approval decision
  - id: approval-check
    type: if
    config:
      condition: "nodes.get(\"human-review\").output.decision == \"approved\""
      true_node: auto-publish
      false_node: notify-rejection
    depends_on: [human-review]

  # Publish the content
  - id: auto-publish
    type: http
    config:
      url: "{{ nodes.validate-input.output.publish_url }}"
      method: POST
      headers:
        Content-Type: application/json
      body:
        title: "{{ nodes.validate-input.output.title }}"
        body: "{{ nodes.validate-input.output.body }}"
        status: published
      credential: cms_api_key
      auth_type: bearer
    depends_on: [route-decision, approval-check]

  # Notify author of rejection
  - id: notify-rejection
    type: email
    config:
      provider: resend
      credential: email_api_key
      to: "{{ nodes.validate-input.output.author }}"
      from: "content-pipeline@company.com"
      subject: "Content review: {{ nodes.validate-input.output.title }}"
      body: |
        Your content "{{ nodes.validate-input.output.title }}" was not approved.

        AI Review: {{ nodes.ai-review.output.suggestion }}

        Please revise and resubmit.
    depends_on: [approval-check]

settings:
  timeout_seconds: 90000  # Allow for 24h approval window
```

---

## Edge Use Cases

These workflows run on constrained devices (Raspberry Pi, NVIDIA Jetson, industrial gateways). They use local Ollama for AI reasoning, require no cloud connectivity for core operation, and buffer results for later upload.

### 4. Smart Factory Anomaly Detection

Runs every 5 minutes on a factory-floor edge device. Reads sensor data, applies threshold checks, and escalates borderline cases to a local LLM for deeper analysis.

```yaml
# Deploy: copy this file + r8r binary to the edge device
# Start: ./r8r server --workflows . --port 3000
# Prereq: ollama serve && ollama pull llama3.2:3b
name: factory-anomaly-detection
description: Real-time sensor monitoring with local AI anomaly analysis
version: 1

triggers:
  - type: cron
    schedule: "*/5 * * * *"  # Every 5 minutes

nodes:
  # Read sensor data from the local sensor gateway API
  # (runs on the same network, no internet needed)
  - id: read-sensors
    type: http
    config:
      url: "http://192.168.1.100:8080/api/sensors/latest"
      method: GET
      timeout_seconds: 10
    retry:
      max_attempts: 2
      delay_seconds: 3
      backoff: linear
    on_error:
      action: fallback
      fallback_value:
        status: "sensor_read_failed"
        sensors: []

  # Check basic thresholds with a deterministic transform
  # (no LLM tokens spent on simple range checks)
  - id: threshold-check
    type: transform
    config:
      expression: |
        let data = input.body;
        let alerts = [];
        // Flag any sensor outside normal operating range
        // Returns: { status: "normal"|"warning"|"critical", alerts: [...] }
        ({
          "status": if data.temperature > 85 { "critical" }
                    else if data.temperature > 75 { "warning" }
                    else { "normal" },
          "temperature": data.temperature,
          "vibration": data.vibration,
          "pressure": data.pressure,
          "alerts": alerts
        })
    depends_on: [read-sensors]

  # Only call the local LLM for borderline/warning cases
  # (saves compute on the edge device)
  - id: needs-analysis
    type: if
    config:
      condition: "nodes.get(\"threshold-check\").output.status == \"warning\""
      true_node: ai-analysis
      false_node: log-result
    depends_on: [threshold-check]

  # Local Ollama analysis -- runs entirely on-device, no cloud needed
  - id: ai-analysis
    type: agent
    config:
      provider: ollama
      model: llama3.2:3b            # Small model for edge devices
      prompt: |
        You are a factory equipment monitoring system.
        Analyze these sensor readings and determine if maintenance is needed.

        Temperature: {{ nodes.threshold-check.output.temperature }}C
        Vibration: {{ nodes.threshold-check.output.vibration }} mm/s
        Pressure: {{ nodes.threshold-check.output.pressure }} bar

        Thresholds were flagged as WARNING.

        Return JSON:
        - anomaly: true or false
        - severity: low, medium, high
        - recommendation: one sentence
        - likely_cause: one sentence
      response_format: json
      timeout_seconds: 60           # Local LLM may be slower
    depends_on: [needs-analysis]

  # Send alert to central monitoring server (buffered if offline)
  - id: send-alert
    type: http
    config:
      url: "{{ env.MONITORING_SERVER_URL }}/api/alerts"
      method: POST
      headers:
        Content-Type: application/json
      body:
        device_id: "{{ env.DEVICE_ID }}"
        timestamp: "{{ $now }}"
        sensor_data: "{{ nodes.threshold-check.output }}"
        ai_analysis: "{{ nodes.ai-analysis.output }}"
      credential: monitoring_api_key
      auth_type: bearer
      timeout_seconds: 10
    depends_on: [ai-analysis]
    on_error:
      continue: true               # Don't fail if cloud is unreachable

  # Log locally regardless of cloud connectivity
  - id: log-result
    type: database
    config:
      db_type: sqlite
      connection_string: "./data/sensor_log.db"
      query: >
        INSERT INTO readings (device_id, temperature, vibration, pressure, status, ai_result, recorded_at)
        VALUES (?, ?, ?, ?, ?, ?, datetime('now'))
      params:
        - "{{ env.DEVICE_ID }}"
        - "{{ nodes.threshold-check.output.temperature }}"
        - "{{ nodes.threshold-check.output.vibration }}"
        - "{{ nodes.threshold-check.output.pressure }}"
        - "{{ nodes.threshold-check.output.status }}"
        - ""
      operation: execute
    depends_on: [threshold-check]

settings:
  timeout_seconds: 120
```

---

### 5. Retail Edge Analytics

Runs daily on an in-store edge device. Aggregates point-of-sale data, generates a summary with a local LLM, and uploads to the cloud when connectivity is available.

```yaml
# Deploy to each store's edge device (mini PC, NUC, etc.)
# Prereq: ollama serve && ollama pull llama3.2:3b
name: retail-edge-analytics
description: Daily POS aggregation with local AI summary and cloud upload
version: 1

triggers:
  - type: cron
    schedule: "0 23 * * *"  # 11 PM daily, after store closes
    timezone: "{{ env.STORE_TIMEZONE }}"

nodes:
  # Fetch today's transactions from the local POS API
  - id: fetch-transactions
    type: http
    config:
      url: "http://localhost:9090/api/transactions?date=today"
      method: GET
      timeout_seconds: 30

  # Aggregate the sales data deterministically
  - id: aggregate-sales
    type: transform
    config:
      expression: |
        let txns = input.body.transactions;
        ({
          "total_revenue": txns.len(),
          "transaction_count": txns.len(),
          "store_id": env.STORE_ID,
          "date": env.TODAY
        })
    depends_on: [fetch-transactions]

  # Deduplicate to prevent duplicate uploads on retry
  - id: dedup-check
    type: database
    config:
      db_type: sqlite
      connection_string: "./data/analytics.db"
      query: >
        SELECT COUNT(*) as count FROM daily_summaries
        WHERE store_id = ? AND summary_date = date('now')
      params:
        - "{{ env.STORE_ID }}"
      operation: query
    depends_on: [aggregate-sales]

  # Generate daily insights with local LLM
  - id: generate-summary
    type: agent
    config:
      provider: ollama
      model: llama3.2:3b
      prompt: |
        You are a retail analytics assistant for store {{ env.STORE_ID }}.
        Generate a brief daily summary from this sales data.

        Sales data: {{ nodes.aggregate-sales.output }}

        Return JSON:
        - headline: one-line summary (e.g., "Strong Tuesday with 15% above average")
        - insights: array of 2-3 bullet points
        - recommendation: one actionable suggestion for tomorrow
      response_format: json
      timeout_seconds: 60
    depends_on: [aggregate-sales]

  # Save summary locally (always succeeds, even offline)
  - id: save-local
    type: database
    config:
      db_type: sqlite
      connection_string: "./data/analytics.db"
      query: >
        INSERT OR REPLACE INTO daily_summaries
        (store_id, summary_date, revenue, transactions, ai_summary, uploaded)
        VALUES (?, date('now'), ?, ?, ?, 0)
      params:
        - "{{ env.STORE_ID }}"
        - "{{ nodes.aggregate-sales.output.total_revenue }}"
        - "{{ nodes.aggregate-sales.output.transaction_count }}"
        - "{{ nodes.generate-summary.output }}"
      operation: execute
    depends_on: [generate-summary]

  # Upload to cloud (best-effort, continues on failure)
  - id: upload-to-cloud
    type: http
    config:
      url: "{{ env.CLOUD_API_URL }}/api/stores/{{ env.STORE_ID }}/daily-summary"
      method: POST
      headers:
        Content-Type: application/json
      body:
        store_id: "{{ env.STORE_ID }}"
        date: "{{ env.TODAY }}"
        sales: "{{ nodes.aggregate-sales.output }}"
        summary: "{{ nodes.generate-summary.output }}"
      credential: cloud_api_key
      auth_type: bearer
      timeout_seconds: 15
    depends_on: [save-local]
    on_error:
      continue: true               # Offline-safe: local copy is the source of truth
    retry:
      max_attempts: 3
      delay_seconds: 30
      backoff: exponential

  # Mark as uploaded if the cloud call succeeded
  - id: mark-uploaded
    type: database
    config:
      db_type: sqlite
      connection_string: "./data/analytics.db"
      query: >
        UPDATE daily_summaries SET uploaded = 1
        WHERE store_id = ? AND summary_date = date('now')
      params:
        - "{{ env.STORE_ID }}"
      operation: execute
    depends_on: [upload-to-cloud]

settings:
  timeout_seconds: 300
```

---

### 6. Agricultural IoT Decision Engine

An hourly workflow running on a field gateway that reads weather and soil sensors, uses a local LLM to decide whether to irrigate, and sends commands to the irrigation controller.

```yaml
# Deploy to a Raspberry Pi or similar field gateway
# Prereq: ollama serve && ollama pull llama3.2:3b
name: irrigation-decision-engine
description: AI-assisted irrigation control using local sensor data
version: 1

triggers:
  - type: cron
    schedule: "0 * * * *"  # Every hour

nodes:
  # Read soil moisture from local sensor hub
  - id: read-soil
    type: http
    config:
      url: "http://192.168.4.1:8080/api/sensors/soil"
      method: GET
      timeout_seconds: 10
    on_error:
      action: fallback
      fallback_value:
        body: { moisture_pct: -1, temperature_c: -1 }

  # Read weather data from local weather station
  - id: read-weather
    type: http
    config:
      url: "http://192.168.4.1:8080/api/sensors/weather"
      method: GET
      timeout_seconds: 10
    on_error:
      action: fallback
      fallback_value:
        body: { humidity_pct: -1, rain_probability: -1, temp_c: -1 }

  # Combine sensor data
  - id: combine-readings
    type: transform
    config:
      expression: |
        ({
          "soil_moisture": nodes.get("read-soil").body.moisture_pct,
          "soil_temp": nodes.get("read-soil").body.temperature_c,
          "humidity": nodes.get("read-weather").body.humidity_pct,
          "rain_probability": nodes.get("read-weather").body.rain_probability,
          "air_temp": nodes.get("read-weather").body.temp_c,
          "timestamp": $now
        })
    depends_on: [read-soil, read-weather]

  # Local LLM decides: irrigate or wait
  - id: decide-irrigation
    type: agent
    config:
      provider: ollama
      model: llama3.2:3b
      prompt: |
        You are an agricultural irrigation advisor.
        Based on these sensor readings, decide whether to irrigate now.

        Soil moisture: {{ nodes.combine-readings.output.soil_moisture }}%
        Soil temperature: {{ nodes.combine-readings.output.soil_temp }}C
        Air humidity: {{ nodes.combine-readings.output.humidity }}%
        Rain probability: {{ nodes.combine-readings.output.rain_probability }}%
        Air temperature: {{ nodes.combine-readings.output.air_temp }}C

        Rules:
        - If soil moisture < 30%, almost always irrigate
        - If rain probability > 70%, prefer to wait
        - Consider time of day and evaporation rates
        - Morning irrigation is preferred over midday

        Return JSON:
        - action: "irrigate" or "wait"
        - duration_minutes: number (0 if waiting)
        - reasoning: one-sentence explanation
        - confidence: 0.0 to 1.0
      response_format: json
      timeout_seconds: 60
    depends_on: [combine-readings]

  # Route based on decision
  - id: check-decision
    type: if
    config:
      condition: "nodes.get(\"decide-irrigation\").output.action == \"irrigate\""
      true_node: activate-irrigation
      false_node: log-decision
    depends_on: [decide-irrigation]

  # Send command to the irrigation controller
  - id: activate-irrigation
    type: http
    config:
      url: "http://192.168.4.50:8080/api/irrigate"
      method: POST
      body:
        zone: "{{ env.IRRIGATION_ZONE }}"
        duration_minutes: "{{ nodes.decide-irrigation.output.duration_minutes }}"
      timeout_seconds: 10
    depends_on: [check-decision]

  # Log every decision with full reasoning to local database
  - id: log-decision
    type: database
    config:
      db_type: sqlite
      connection_string: "./data/irrigation.db"
      query: >
        INSERT INTO decisions
        (timestamp, soil_moisture, rain_prob, action, duration, reasoning, confidence)
        VALUES (datetime('now'), ?, ?, ?, ?, ?, ?)
      params:
        - "{{ nodes.combine-readings.output.soil_moisture }}"
        - "{{ nodes.combine-readings.output.rain_probability }}"
        - "{{ nodes.decide-irrigation.output.action }}"
        - "{{ nodes.decide-irrigation.output.duration_minutes }}"
        - "{{ nodes.decide-irrigation.output.reasoning }}"
        - "{{ nodes.decide-irrigation.output.confidence }}"
      operation: execute
    depends_on: [decide-irrigation]

settings:
  timeout_seconds: 180
```

---

## Hybrid Use Cases (Edge + Cloud)

### 7. Distributed Sensor Fleet with Cloud Aggregation

**Architecture**: Each edge device runs r8r locally with a processing workflow that filters noise and buffers results. A central cloud r8r instance receives pre-processed data via webhooks and runs fleet-wide analysis.

```
  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐
  │  Edge Site A  │  │  Edge Site B  │  │  Edge Site C  │
  │  r8r + Ollama │  │  r8r + Ollama │  │  r8r + Ollama │
  │  local sensor │  │  local sensor │  │  local sensor │
  │  processing   │  │  processing   │  │  processing   │
  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘
         │                  │                  │
         │  HTTPS POST      │  HTTPS POST      │  HTTPS POST
         │  (when online)   │  (when online)   │  (when online)
         ▼                  ▼                  ▼
  ┌─────────────────────────────────────────────────┐
  │              Cloud r8r Instance                  │
  │  webhook trigger → aggregate → analyze → report │
  └─────────────────────────────────────────────────┘
```

**Edge workflow** (deployed to each site):

```yaml
name: edge-sensor-processor
description: Local sensor processing with cloud upload
version: 1

triggers:
  - type: cron
    schedule: "*/10 * * * *"  # Every 10 minutes

nodes:
  - id: read-sensors
    type: http
    config:
      url: "http://localhost:9090/api/sensors/all"
      method: GET
      timeout_seconds: 10

  # Filter out noise locally -- no cloud cost for normal readings
  - id: filter-significant
    type: transform
    config:
      expression: |
        let readings = input.body.readings;
        // Only forward readings outside normal range
        ({
          "site_id": env.SITE_ID,
          "timestamp": $now,
          "readings": readings,
          "reading_count": readings.len()
        })
    depends_on: [read-sensors]

  # Quick local analysis for anything unusual
  - id: local-analysis
    type: agent
    config:
      provider: ollama
      model: llama3.2:3b
      prompt: |
        Summarize these sensor readings in one sentence.
        Flag anything unusual.
        Data: {{ nodes.filter-significant.output.readings }}
        Return JSON: { "summary": "...", "unusual": true/false }
      response_format: json
      timeout_seconds: 45
    depends_on: [filter-significant]

  # Upload to central cloud (best-effort)
  - id: upload-to-cloud
    type: http
    config:
      url: "{{ env.CLOUD_R8R_URL }}/webhooks/fleet-ingest"
      method: POST
      body:
        site_id: "{{ env.SITE_ID }}"
        readings: "{{ nodes.filter-significant.output }}"
        local_analysis: "{{ nodes.local-analysis.output }}"
      credential: fleet_api_key
      auth_type: bearer
      timeout_seconds: 15
    depends_on: [local-analysis]
    on_error:
      continue: true

  # Always save locally
  - id: save-local
    type: database
    config:
      db_type: sqlite
      connection_string: "./data/readings.db"
      query: >
        INSERT INTO sensor_data (site_id, readings, analysis, uploaded, recorded_at)
        VALUES (?, ?, ?, ?, datetime('now'))
      params:
        - "{{ env.SITE_ID }}"
        - "{{ nodes.filter-significant.output }}"
        - "{{ nodes.local-analysis.output }}"
        - "1"
      operation: execute
    depends_on: [local-analysis]

settings:
  timeout_seconds: 120
```

**Cloud workflow** (runs on the central server):

```yaml
name: fleet-ingest
description: Aggregate and analyze data from edge sensor fleet
version: 1

triggers:
  - type: webhook
    config:
      path: /fleet-ingest
      method: POST

nodes:
  # Store incoming edge data
  - id: store-reading
    type: database
    config:
      db_type: postgres
      connection_string: "{{ env.DATABASE_URL }}"
      query: >
        INSERT INTO fleet_readings (site_id, readings, local_analysis, received_at)
        VALUES ($1, $2, $3, NOW())
      params:
        - "{{ input.site_id }}"
        - "{{ input.readings }}"
        - "{{ input.local_analysis }}"
      operation: execute

  # Check if we have enough data for fleet-wide analysis
  - id: check-batch
    type: database
    config:
      db_type: postgres
      connection_string: "{{ env.DATABASE_URL }}"
      query: >
        SELECT COUNT(DISTINCT site_id) as sites,
               COUNT(*) as readings
        FROM fleet_readings
        WHERE received_at > NOW() - INTERVAL '1 hour'
      operation: query
    depends_on: [store-reading]

  # Run fleet-wide analysis when we have data from multiple sites
  - id: should-analyze
    type: if
    config:
      condition: "nodes.get(\"check-batch\").output.rows[0].sites >= 3"
      true_node: fleet-analysis
      false_node: skip-analysis
    depends_on: [check-batch]

  - id: fleet-analysis
    type: agent
    config:
      provider: anthropic
      model: claude-sonnet-4-5-20250929
      credential: anthropic
      system: "You are a fleet-wide sensor analysis system."
      prompt: |
        Analyze the latest readings from all edge sites.
        Look for cross-site patterns and correlations.

        Latest data: {{ nodes.check-batch.output.rows }}

        Return JSON:
        - fleet_status: "normal", "attention", or "critical"
        - cross_site_patterns: array of observations
        - recommendations: array of action items
      response_format: json
      max_tokens: 2048
    depends_on: [should-analyze]

  - id: notify-if-critical
    type: slack
    config:
      channel: "#fleet-ops"
      credential: slack_token
      text: |
        *Fleet Sensor Alert*
        Status: {{ nodes.fleet-analysis.output.fleet_status }}
        {{ nodes.fleet-analysis.output.recommendations }}
    depends_on: [fleet-analysis]

  - id: skip-analysis
    type: debug
    config:
      label: "Not enough sites reporting yet, skipping fleet analysis"
      level: info
    depends_on: [should-analyze]

settings:
  timeout_seconds: 60
```

---

### 8. Robot Fleet with Central Oversight

**Architecture**: Each robot runs a local r8r instance (future: `r8r-runner`) with decision workflows. A local Ollama model handles real-time navigation and task decisions. Execution traces are uploaded to a central r8r server for monitoring. High-risk actions require human approval through the MCP approval node.

> **Note**: `r8r-runner` is **planned** (see [Runner Plan](RUNNER_PLAN.md)). Today you can achieve this by running a full `r8r server` on each robot. The runner will add signed bundle sync, policy enforcement, and lighter footprint.

```
  ┌───────────────────────┐
  │   Central r8r Server  │
  │   (cloud or on-prem)  │
  │   - Fleet dashboard   │
  │   - Approval queue    │
  │   - Trace aggregation │
  └───────────┬───────────┘
              │
    ┌─────────┼──────────┐
    │         │          │
    ▼         ▼          ▼
  ┌─────┐  ┌─────┐  ┌─────┐
  │Bot A│  │Bot B│  │Bot C│
  │r8r  │  │r8r  │  │r8r  │
  │Ollama│ │Ollama│ │Ollama│
  └─────┘  └─────┘  └─────┘
```

**Local robot workflow**:

```yaml
# Runs on each robot's onboard computer
# Prereq: ollama serve && ollama pull llama3.2:3b
name: robot-task-executor
description: Local robot decision loop with central oversight
version: 1

triggers:
  - type: cron
    schedule: "*/1 * * * *"  # Every minute

nodes:
  # Read current robot state from local sensors
  - id: read-state
    type: http
    config:
      url: "http://localhost:5000/api/robot/state"
      method: GET
      timeout_seconds: 5

  # Get next task from local queue
  - id: get-task
    type: http
    config:
      url: "http://localhost:5000/api/tasks/next"
      method: GET
      timeout_seconds: 5
    depends_on: [read-state]

  # Local LLM decides how to execute the task
  - id: plan-execution
    type: agent
    config:
      provider: ollama
      model: llama3.2:3b
      prompt: |
        You are a robot task planner.

        Current state:
        - Position: {{ nodes.read-state.output.body.position }}
        - Battery: {{ nodes.read-state.output.body.battery_pct }}%
        - Payload: {{ nodes.read-state.output.body.payload }}

        Next task: {{ nodes.get-task.output.body }}

        Plan the execution. Return JSON:
        - feasible: true/false
        - risk_level: "low", "medium", "high"
        - steps: array of movement/action commands
        - estimated_minutes: number
        - requires_approval: true if entering restricted zone or battery < 20%
      response_format: json
      timeout_seconds: 30
    depends_on: [get-task]

  # Gate high-risk actions through human approval
  - id: check-risk
    type: if
    config:
      condition: "nodes.get(\"plan-execution\").output.requires_approval == true"
      true_node: request-approval
      false_node: execute-plan
    depends_on: [plan-execution]

  # Approval pauses execution until a human responds via MCP
  - id: request-approval
    type: approval
    config:
      title: "Robot {{ env.ROBOT_ID }}: high-risk action"
      description: |
        Task: {{ nodes.get-task.output.body.description }}
        Risk: {{ nodes.plan-execution.output.risk_level }}
        Steps: {{ nodes.plan-execution.output.steps }}
      timeout_seconds: 600         # 10-minute window
      default_action: reject        # Safe default: don't proceed
    depends_on: [check-risk]

  # Execute the approved plan
  - id: execute-plan
    type: http
    config:
      url: "http://localhost:5000/api/robot/execute"
      method: POST
      body:
        steps: "{{ nodes.plan-execution.output.steps }}"
        task_id: "{{ nodes.get-task.output.body.task_id }}"
      timeout_seconds: 30
    depends_on: [check-risk, request-approval]

  # Upload execution trace to central server (best-effort)
  - id: report-to-central
    type: http
    config:
      url: "{{ env.CENTRAL_SERVER_URL }}/api/fleet/{{ env.ROBOT_ID }}/report"
      method: POST
      body:
        robot_id: "{{ env.ROBOT_ID }}"
        task: "{{ nodes.get-task.output.body }}"
        plan: "{{ nodes.plan-execution.output }}"
        result: "{{ nodes.execute-plan.output }}"
        timestamp: "{{ $now }}"
      credential: fleet_api_key
      auth_type: bearer
      timeout_seconds: 10
    depends_on: [execute-plan]
    on_error:
      continue: true               # Offline-safe: robot keeps working

settings:
  timeout_seconds: 900             # Allow for approval wait time
```

---

## MCP Agent Use Cases

r8r exposes 15 MCP tools that let AI coding agents (Claude Code, Cursor, Windsurf, etc.) build, test, deploy, and monitor workflows through natural conversation.

### 9. Claude Code as Automation Builder

An AI coding agent uses r8r's MCP server to create and iterate on workflows entirely through tool calls.

**Step 1: Generate a workflow from natural language**

```json
// Tool: r8r_generate
{
  "description": "Every morning at 9am, check our Stripe dashboard for failed payments from the last 24 hours, then send a summary to the #billing Slack channel",
  "provider": "openai"
}
// Returns: generated YAML + validation status
```

**Step 2: Lint the generated workflow**

```json
// Tool: r8r_lint
{
  "workflow_yaml": "<the generated YAML from step 1>"
}
// Returns: validation errors with suggestions, or "valid"
```

**Step 3: Test with mock data before deploying**

```json
// Tool: r8r_test
{
  "workflow_yaml": "<the YAML with pinned_data for mocking Stripe/Slack>",
  "input": {},
  "expected_output": { "failed_payments": 3, "slack_sent": true },
  "mode": "contains"
}
// Returns: pass/fail + actual output
```

**Step 4: Deploy to the running r8r instance**

```json
// Tool: r8r_create_workflow
{
  "name": "billing-failed-payments",
  "definition": "<the validated YAML>"
}
// Returns: workflow ID, node count, trigger count
```

**Step 5: Execute and verify**

```json
// Tool: r8r_run_and_wait
{
  "workflow": "billing-failed-payments",
  "input": {}
}
// Returns: workflow output directly
```

**Step 6: Check the execution trace**

```json
// Tool: r8r_get_trace
{
  "execution_id": "exec-abc123"
}
// Returns: per-node timing, status, and errors
```

**Step 7: Discover what a workflow expects**

```json
// Tool: r8r_discover
{
  "workflow": "billing-failed-payments"
}
// Returns: input schema, parameters, node list, trigger config
```

The full cycle -- generate, lint, test, deploy, run, inspect -- happens in a single conversation without the developer writing any YAML by hand.

---

### 10. Multi-Agent Orchestration

Multiple AI agents coordinate through r8r workflows, using it as a shared execution backbone.

**Architecture**:

```
  Agent A (Data Scout)              Agent B (Analyst)
  ┌──────────────────┐              ┌──────────────────┐
  │ Monitors RSS,    │   webhook    │ Deep analysis,   │
  │ APIs, social     │──────────────│ decision-making  │
  │ for signals      │   trigger    │ via agent node   │
  └──────────────────┘              └──────┬───────────┘
         │                                  │
         │  r8r cron + HTTP                │  r8r stores results
         │                                  │
         ▼                                  ▼
  ┌─────────────────────────────────────────────────────┐
  │                   r8r Server                         │
  │  Shared execution, storage, and API layer            │
  │  Any agent can query results via r8r API             │
  └─────────────────────────────────────────────────────┘
```

**Data Scout workflow** (Agent A -- monitors sources on a schedule):

```yaml
name: data-scout
description: Monitor data sources for interesting signals
version: 1

triggers:
  - type: cron
    schedule: "0 */2 * * *"  # Every 2 hours

nodes:
  # Scan multiple data sources
  - id: fetch-hackernews
    type: http
    config:
      url: "https://hacker-news.firebaseio.com/v0/topstories.json"
      method: GET
      timeout_seconds: 15

  - id: fetch-reddit
    type: http
    config:
      url: "https://www.reddit.com/r/technology/top.json?limit=20&t=day"
      method: GET
      headers:
        User-Agent: "r8r-data-scout/1.0"
      timeout_seconds: 15

  # AI filters for relevance to our domain
  - id: filter-relevant
    type: agent
    config:
      provider: openai
      model: gpt-4o
      credential: openai
      prompt: |
        You are a technology trend scout.
        Our focus areas: AI/ML, developer tools, automation, edge computing.

        HackerNews top stories: {{ nodes.fetch-hackernews.output.body }}
        Reddit r/technology: {{ nodes.fetch-reddit.output.body }}

        Return JSON:
        - signals: array of {source, title, url, relevance_score, brief}
          (only items with relevance_score > 0.7)
        - trend_summary: one paragraph on emerging themes
      response_format: json
      max_tokens: 2048
    depends_on: [fetch-hackernews, fetch-reddit]

  # Store signals in the shared database
  - id: store-signals
    type: database
    config:
      db_type: sqlite
      connection_string: "./data/signals.db"
      query: >
        INSERT INTO signals (source, title, url, relevance, brief, discovered_at)
        VALUES (?, ?, ?, ?, ?, datetime('now'))
      params:
        - "multi"
        - "{{ nodes.filter-relevant.output.trend_summary }}"
        - ""
        - "0"
        - "{{ nodes.filter-relevant.output.signals }}"
      operation: execute
    depends_on: [filter-relevant]

  # Trigger Agent B if high-value signals found
  - id: check-high-value
    type: if
    config:
      condition: "nodes.get(\"filter-relevant\").output.signals.len() > 0"
      true_node: trigger-analyst
      false_node: log-quiet
    depends_on: [filter-relevant]

  - id: trigger-analyst
    type: http
    config:
      url: "http://localhost:3000/webhooks/analyst-trigger"
      method: POST
      body:
        signals: "{{ nodes.filter-relevant.output.signals }}"
        trend_summary: "{{ nodes.filter-relevant.output.trend_summary }}"
      timeout_seconds: 10
    depends_on: [check-high-value]

  - id: log-quiet
    type: debug
    config:
      label: "No high-value signals this cycle"
      level: info
    depends_on: [check-high-value]

settings:
  timeout_seconds: 120
```

**Analyst workflow** (Agent B -- triggered by Agent A's findings):

```yaml
name: analyst-trigger
description: Deep analysis of signals detected by the data scout
version: 1

triggers:
  - type: webhook
    config:
      path: /analyst-trigger
      method: POST

nodes:
  # Deep analysis with a more capable model
  - id: deep-analysis
    type: agent
    config:
      provider: anthropic
      model: claude-sonnet-4-5-20250929
      credential: anthropic
      system: "You are a strategic technology analyst. Provide actionable insights."
      prompt: |
        The data scout detected these signals:
        {{ input.signals }}

        Trend summary: {{ input.trend_summary }}

        Perform deep analysis:
        1. Which signals represent real opportunities vs noise?
        2. What are the implications for our product roadmap?
        3. Are there any threats we should respond to?

        Return JSON:
        - opportunities: array of {signal, analysis, urgency, recommended_action}
        - threats: array of {signal, analysis, urgency}
        - roadmap_implications: string (one paragraph)
      response_format: json
      max_tokens: 4096

  # Store the analysis
  - id: store-analysis
    type: database
    config:
      db_type: sqlite
      connection_string: "./data/signals.db"
      query: >
        INSERT INTO analyses (signals_summary, opportunities, threats, implications, analyzed_at)
        VALUES (?, ?, ?, ?, datetime('now'))
      params:
        - "{{ input.trend_summary }}"
        - "{{ nodes.deep-analysis.output.opportunities }}"
        - "{{ nodes.deep-analysis.output.threats }}"
        - "{{ nodes.deep-analysis.output.roadmap_implications }}"
      operation: execute
    depends_on: [deep-analysis]

  # Notify the team about urgent findings
  - id: notify-team
    type: slack
    config:
      channel: "#product-intel"
      credential: slack_token
      blocks:
        - type: section
          text:
            type: mrkdwn
            text: |
              *Technology Intelligence Report*

              *Roadmap Implications:*
              {{ nodes.deep-analysis.output.roadmap_implications }}

              *Opportunities:* {{ nodes.deep-analysis.output.opportunities }}
              *Threats:* {{ nodes.deep-analysis.output.threats }}
    depends_on: [store-analysis]

settings:
  timeout_seconds: 120
```

Both workflows share the same r8r instance and SQLite database. Any agent -- or human -- can query stored signals and analyses through the r8r API or MCP tools at any time.

---

## Summary

| # | Use Case | Deployment | Trigger | AI Provider | Key Nodes |
|---|----------|------------|---------|-------------|-----------|
| 1 | GitHub Issue Triage | Cloud | Webhook | OpenAI | agent, http, slack |
| 2 | Competitive Price Monitor | Cloud | Cron | Anthropic | http, agent, database, slack, if |
| 3 | Content Pipeline | Cloud | Webhook | OpenAI | agent, approval, http, email, if |
| 4 | Factory Anomaly Detection | Edge | Cron | Ollama | http, transform, agent, database, if |
| 5 | Retail Edge Analytics | Edge | Cron | Ollama | http, agent, database, dedupe |
| 6 | Irrigation Decision Engine | Edge | Cron | Ollama | http, agent, database, if |
| 7 | Distributed Sensor Fleet | Hybrid | Cron + Webhook | Ollama + Anthropic | http, agent, database, if |
| 8 | Robot Fleet Oversight | Hybrid | Cron | Ollama | http, agent, approval, database |
| 9 | Claude Code Builder | MCP | Manual | (via MCP tools) | All MCP tools |
| 10 | Multi-Agent Orchestration | Cloud | Cron + Webhook | OpenAI + Anthropic | http, agent, database, slack, if |
