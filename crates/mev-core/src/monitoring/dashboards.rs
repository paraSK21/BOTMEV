//! Grafana dashboard configurations for MEV bot monitoring

use serde_json::{json, Value};

/// Generate Grafana dashboard JSON for MEV bot monitoring
pub fn generate_mev_dashboard() -> Value {
    json!({
        "dashboard": {
            "id": null,
            "title": "MEV Bot Production Dashboard",
            "tags": ["mev", "ethereum", "trading"],
            "timezone": "browser",
            "panels": [
                generate_overview_panel(),
                generate_latency_panel(),
                generate_bundle_success_panel(),
                generate_pnl_panel(),
                generate_system_health_panel(),
                generate_error_tracking_panel()
            ],
            "time": {
                "from": "now-1h",
                "to": "now"
            },
            "timepicker": {},
            "templating": {
                "list": [
                    {
                        "name": "strategy",
                        "type": "query",
                        "query": "label_values(mev_bundles_created_total, strategy)",
                        "refresh": 1,
                        "includeAll": true,
                        "multi": true
                    }
                ]
            },
            "annotations": {
                "list": []
            },
            "refresh": "5s",
            "schemaVersion": 30,
            "version": 1
        }
    })
}

fn generate_overview_panel() -> Value {
    json!({
        "id": 1,
        "title": "MEV Bot Overview",
        "type": "stat",
        "targets": [
            {
                "expr": "rate(mev_bundles_created_total[5m])",
                "legendFormat": "Bundles/sec"
            },
            {
                "expr": "rate(mev_opportunities_detected_total[5m])",
                "legendFormat": "Opportunities/sec"
            },
            {
                "expr": "mev_simulations_per_second",
                "legendFormat": "Simulations/sec"
            }
        ],
        "gridPos": {
            "h": 8,
            "w": 12,
            "x": 0,
            "y": 0
        },
        "options": {
            "colorMode": "value",
            "graphMode": "area",
            "justifyMode": "auto",
            "orientation": "horizontal"
        }
    })
}

fn generate_latency_panel() -> Value {
    json!({
        "id": 2,
        "title": "Detection & Decision Latency",
        "type": "graph",
        "targets": [
            {
                "expr": "histogram_quantile(0.50, rate(mev_detection_latency_seconds_bucket[5m]))",
                "legendFormat": "Detection P50"
            },
            {
                "expr": "histogram_quantile(0.95, rate(mev_detection_latency_seconds_bucket[5m]))",
                "legendFormat": "Detection P95"
            },
            {
                "expr": "histogram_quantile(0.50, rate(mev_decision_latency_seconds_bucket[5m]))",
                "legendFormat": "Decision P50"
            },
            {
                "expr": "histogram_quantile(0.95, rate(mev_decision_latency_seconds_bucket[5m]))",
                "legendFormat": "Decision P95"
            }
        ],
        "gridPos": {
            "h": 8,
            "w": 12,
            "x": 12,
            "y": 0
        },
        "yAxes": [
            {
                "label": "Seconds",
                "min": 0
            }
        ],
        "xAxis": {
            "mode": "time"
        },
        "legend": {
            "show": true,
            "values": true,
            "current": true
        }
    })
}

fn generate_bundle_success_panel() -> Value {
    json!({
        "id": 3,
        "title": "Bundle Success Heatmap",
        "type": "heatmap",
        "targets": [
            {
                "expr": "rate(mev_bundles_included_total[5m]) / rate(mev_bundles_submitted_total[5m])",
                "legendFormat": "Success Rate by Strategy"
            }
        ],
        "gridPos": {
            "h": 8,
            "w": 24,
            "x": 0,
            "y": 8
        },
        "heatmap": {
            "xAxis": {
                "show": true
            },
            "yAxis": {
                "show": true,
                "label": "Strategy"
            },
            "colorMode": "spectrum",
            "colorScale": "exponential",
            "colorScheme": "interpolateSpectral"
        }
    })
}

fn generate_pnl_panel() -> Value {
    json!({
        "id": 4,
        "title": "Profit & Loss Curves",
        "type": "graph",
        "targets": [
            {
                "expr": "sum(rate(mev_realized_pnl_wei[5m])) by (strategy)",
                "legendFormat": "Realized PnL - {{strategy}}"
            },
            {
                "expr": "sum(rate(mev_simulated_pnl_wei[5m])) by (strategy)",
                "legendFormat": "Simulated PnL - {{strategy}}"
            },
            {
                "expr": "sum(rate(mev_gas_costs_wei_total[5m]))",
                "legendFormat": "Gas Costs"
            }
        ],
        "gridPos": {
            "h": 8,
            "w": 12,
            "x": 0,
            "y": 16
        },
        "yAxes": [
            {
                "label": "Wei/sec",
                "min": null
            }
        ],
        "seriesOverrides": [
            {
                "alias": "Gas Costs",
                "yAxis": 2,
                "color": "red"
            }
        ]
    })
}

fn generate_system_health_panel() -> Value {
    json!({
        "id": 5,
        "title": "System Health",
        "type": "graph",
        "targets": [
            {
                "expr": "mev_cpu_usage_percent",
                "legendFormat": "CPU Usage %"
            },
            {
                "expr": "mev_memory_usage_bytes / 1024 / 1024",
                "legendFormat": "Memory Usage MB"
            },
            {
                "expr": "mev_mempool_transactions_per_second",
                "legendFormat": "Mempool TPS"
            }
        ],
        "gridPos": {
            "h": 8,
            "w": 12,
            "x": 12,
            "y": 16
        },
        "yAxes": [
            {
                "label": "Percentage / MB / TPS",
                "min": 0
            }
        ]
    })
}

fn generate_error_tracking_panel() -> Value {
    json!({
        "id": 6,
        "title": "Error Tracking",
        "type": "table",
        "targets": [
            {
                "expr": "topk(10, rate(mev_errors_total[5m]))",
                "legendFormat": "{{error_type}} - {{component}}"
            },
            {
                "expr": "topk(10, rate(mev_failures_by_type_total[5m]))",
                "legendFormat": "{{failure_type}} - {{recovery_action}}"
            }
        ],
        "gridPos": {
            "h": 8,
            "w": 24,
            "x": 0,
            "y": 24
        },
        "columns": [
            {
                "text": "Error Type",
                "value": "error_type"
            },
            {
                "text": "Component",
                "value": "component"
            },
            {
                "text": "Rate/sec",
                "value": "Value"
            }
        ]
    })
}

/// Generate alerting rules for Grafana
pub fn generate_alerting_rules() -> Value {
    json!({
        "groups": [
            {
                "name": "mev_bot_alerts",
                "rules": [
                    {
                        "alert": "MEVBotHighLatency",
                        "expr": "histogram_quantile(0.95, rate(mev_detection_latency_seconds_bucket[5m])) > 0.1",
                        "for": "2m",
                        "labels": {
                            "severity": "warning"
                        },
                        "annotations": {
                            "summary": "MEV bot detection latency is high",
                            "description": "95th percentile detection latency is {{ $value }}s"
                        }
                    },
                    {
                        "alert": "MEVBotLowSuccessRate",
                        "expr": "rate(mev_bundles_included_total[5m]) / rate(mev_bundles_submitted_total[5m]) < 0.5",
                        "for": "5m",
                        "labels": {
                            "severity": "critical"
                        },
                        "annotations": {
                            "summary": "MEV bot bundle success rate is low",
                            "description": "Bundle success rate is {{ $value | humanizePercentage }}"
                        }
                    },
                    {
                        "alert": "MEVBotHighErrorRate",
                        "expr": "rate(mev_errors_total[5m]) > 10",
                        "for": "1m",
                        "labels": {
                            "severity": "warning"
                        },
                        "annotations": {
                            "summary": "MEV bot error rate is high",
                            "description": "Error rate is {{ $value }} errors/sec"
                        }
                    },
                    {
                        "alert": "MEVBotKillSwitchActivated",
                        "expr": "increase(mev_kill_switch_activations_total[1m]) > 0",
                        "for": "0s",
                        "labels": {
                            "severity": "critical"
                        },
                        "annotations": {
                            "summary": "MEV bot kill switch has been activated",
                            "description": "Kill switch activated - immediate attention required"
                        }
                    },
                    {
                        "alert": "MEVBotHighMemoryUsage",
                        "expr": "mev_memory_usage_bytes > 2147483648",
                        "for": "5m",
                        "labels": {
                            "severity": "warning"
                        },
                        "annotations": {
                            "summary": "MEV bot memory usage is high",
                            "description": "Memory usage is {{ $value | humanizeBytes }}"
                        }
                    }
                ]
            }
        ]
    })
}

/// Export dashboard configuration to file
pub fn export_dashboard_config(output_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let dashboard = generate_mev_dashboard();
    let dashboard_json = serde_json::to_string_pretty(&dashboard)?;
    std::fs::write(output_path, dashboard_json)?;
    Ok(())
}

/// Export alerting rules to file
pub fn export_alerting_rules(output_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let rules = generate_alerting_rules();
    let rules_yaml = serde_yaml::to_string(&rules)?;
    std::fs::write(output_path, rules_yaml)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_dashboard_generation() {
        let dashboard = generate_mev_dashboard();
        
        // Verify basic structure
        assert!(dashboard["dashboard"]["title"].is_string());
        assert!(dashboard["dashboard"]["panels"].is_array());
        
        let panels = dashboard["dashboard"]["panels"].as_array().unwrap();
        assert_eq!(panels.len(), 6);
        
        // Verify panel types
        assert_eq!(panels[0]["type"], "stat");
        assert_eq!(panels[1]["type"], "graph");
        assert_eq!(panels[2]["type"], "heatmap");
    }
    
    #[test]
    fn test_alerting_rules_generation() {
        let rules = generate_alerting_rules();
        
        // Verify structure
        assert!(rules["groups"].is_array());
        
        let groups = rules["groups"].as_array().unwrap();
        assert_eq!(groups.len(), 1);
        
        let mev_group = &groups[0];
        assert_eq!(mev_group["name"], "mev_bot_alerts");
        
        let rules_list = mev_group["rules"].as_array().unwrap();
        assert_eq!(rules_list.len(), 5);
        
        // Verify alert names
        let alert_names: Vec<&str> = rules_list
            .iter()
            .map(|rule| rule["alert"].as_str().unwrap())
            .collect();
        
        assert!(alert_names.contains(&"MEVBotHighLatency"));
        assert!(alert_names.contains(&"MEVBotLowSuccessRate"));
        assert!(alert_names.contains(&"MEVBotKillSwitchActivated"));
    }
}