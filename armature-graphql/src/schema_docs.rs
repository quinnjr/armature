/// Generate HTML documentation for GraphQL schema
use async_graphql::Schema;

/// Generate HTML documentation page for GraphQL schema
///
/// This generates a comprehensive, interactive documentation page
/// similar to GraphQL Voyager or GraphQL Docs Explorer.
///
/// # Example
///
/// ```rust,no_run
/// use armature_graphql::{Schema, EmptyMutation, EmptySubscription};
/// use armature_graphql::schema_docs::generate_schema_docs_html;
///
/// #[derive(async_graphql::SimpleObject)]
/// struct User {
///     id: i32,
///     name: String,
/// }
///
/// struct Query;
///
/// #[async_graphql::Object]
/// impl Query {
///     async fn user(&self, id: i32) -> User {
///         User { id, name: "Alice".to_string() }
///     }
/// }
///
/// # async fn example() {
/// let schema = Schema::new(Query, EmptyMutation, EmptySubscription);
/// let html = generate_schema_docs_html(&schema, "/graphql", "My API");
/// # }
/// ```
pub fn generate_schema_docs_html<Query, Mutation, Subscription>(
    schema: &Schema<Query, Mutation, Subscription>,
    endpoint: &str,
    title: &str,
) -> String
where
    Query: async_graphql::ObjectType + 'static,
    Mutation: async_graphql::ObjectType + 'static,
    Subscription: async_graphql::SubscriptionType + 'static,
{
    let sdl = schema.sdl();

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{title} - GraphQL Schema Documentation</title>
    <style>
        * {{
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }}

        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, Cantarell, sans-serif;
            line-height: 1.6;
            color: #333;
            background: #f5f5f5;
        }}

        .container {{
            max-width: 1200px;
            margin: 0 auto;
            padding: 20px;
        }}

        header {{
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            color: white;
            padding: 40px 20px;
            margin-bottom: 30px;
            border-radius: 8px;
            box-shadow: 0 4px 6px rgba(0, 0, 0, 0.1);
        }}

        h1 {{
            font-size: 2.5em;
            margin-bottom: 10px;
        }}

        .subtitle {{
            font-size: 1.2em;
            opacity: 0.9;
        }}

        .endpoint {{
            background: rgba(255, 255, 255, 0.2);
            padding: 10px 20px;
            border-radius: 4px;
            display: inline-block;
            margin-top: 15px;
            font-family: 'Courier New', monospace;
        }}

        .tabs {{
            display: flex;
            gap: 10px;
            margin-bottom: 20px;
            border-bottom: 2px solid #ddd;
        }}

        .tab {{
            padding: 12px 24px;
            background: white;
            border: none;
            border-radius: 8px 8px 0 0;
            cursor: pointer;
            font-size: 16px;
            transition: all 0.3s;
            border: 2px solid transparent;
            border-bottom: none;
        }}

        .tab:hover {{
            background: #f0f0f0;
        }}

        .tab.active {{
            background: white;
            border-color: #667eea;
            color: #667eea;
            font-weight: 600;
        }}

        .tab-content {{
            display: none;
            background: white;
            padding: 30px;
            border-radius: 0 8px 8px 8px;
            box-shadow: 0 2px 4px rgba(0, 0, 0, 0.1);
        }}

        .tab-content.active {{
            display: block;
        }}

        .schema-viewer {{
            background: #2d2d2d;
            color: #f8f8f2;
            padding: 20px;
            border-radius: 8px;
            overflow-x: auto;
            font-family: 'Courier New', monospace;
            font-size: 14px;
            line-height: 1.5;
        }}

        .keyword {{
            color: #ff79c6;
            font-weight: bold;
        }}

        .type {{
            color: #8be9fd;
        }}

        .field {{
            color: #50fa7b;
        }}

        .description {{
            color: #6272a4;
            font-style: italic;
        }}

        .info-box {{
            background: #e3f2fd;
            border-left: 4px solid #2196f3;
            padding: 15px 20px;
            margin: 20px 0;
            border-radius: 4px;
        }}

        .info-box h3 {{
            color: #1976d2;
            margin-bottom: 10px;
        }}

        .features {{
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));
            gap: 20px;
            margin-top: 30px;
        }}

        .feature-card {{
            background: white;
            padding: 20px;
            border-radius: 8px;
            box-shadow: 0 2px 4px rgba(0, 0, 0, 0.1);
            border-left: 4px solid #667eea;
        }}

        .feature-card h3 {{
            color: #667eea;
            margin-bottom: 10px;
        }}

        .copy-button {{
            background: #667eea;
            color: white;
            border: none;
            padding: 10px 20px;
            border-radius: 4px;
            cursor: pointer;
            font-size: 14px;
            margin-top: 10px;
            transition: background 0.3s;
        }}

        .copy-button:hover {{
            background: #764ba2;
        }}

        .copy-button:active {{
            transform: scale(0.98);
        }}

        footer {{
            text-align: center;
            padding: 20px;
            color: #666;
            margin-top: 40px;
        }}

        a {{
            color: #667eea;
            text-decoration: none;
        }}

        a:hover {{
            text-decoration: underline;
        }}

        code {{
            background: #f4f4f4;
            padding: 2px 6px;
            border-radius: 3px;
            font-family: 'Courier New', monospace;
        }}
    </style>
</head>
<body>
    <div class="container">
        <header>
            <h1>📚 {title}</h1>
            <p class="subtitle">GraphQL Schema Documentation</p>
            <div class="endpoint">Endpoint: {endpoint}</div>
        </header>

        <div class="tabs">
            <button class="tab active" onclick="switchTab('overview')">Overview</button>
            <button class="tab" onclick="switchTab('schema')">Schema (SDL)</button>
            <button class="tab" onclick="switchTab('playground')">Try It Out</button>
        </div>

        <div id="overview" class="tab-content active">
            <h2>📖 About This API</h2>
            <p>This page provides comprehensive documentation for the GraphQL API. Explore the schema, types, queries, mutations, and subscriptions available.</p>

            <div class="info-box">
                <h3>🚀 Getting Started</h3>
                <p>To interact with this GraphQL API, send POST requests to <code>{endpoint}</code> with a JSON body containing your GraphQL query.</p>
                <p><strong>Example:</strong></p>
                <pre style="background: #f9f9f9; padding: 15px; border-radius: 4px; overflow-x: auto;">
POST {endpoint}
Content-Type: application/json

{{
  "query": "{{ __schema {{ queryType {{ name }} }} }}"
}}
                </pre>
            </div>

            <h2>✨ Features</h2>
            <div class="features">
                <div class="feature-card">
                    <h3>🔍 Type-Safe Queries</h3>
                    <p>All queries and mutations are strongly typed, providing compile-time safety and excellent IDE support.</p>
                </div>
                <div class="feature-card">
                    <h3>📝 Introspection</h3>
                    <p>Full schema introspection support allows tools like GraphQL Playground and GraphiQL to provide auto-completion.</p>
                </div>
                <div class="feature-card">
                    <h3>🔄 Real-time Subscriptions</h3>
                    <p>WebSocket-based subscriptions enable real-time updates and live data streaming.</p>
                </div>
                <div class="feature-card">
                    <h3>⚡ High Performance</h3>
                    <p>Built on Rust and async-graphql for blazing-fast query execution and minimal overhead.</p>
                </div>
            </div>
        </div>

        <div id="schema" class="tab-content">
            <h2>📄 Schema Definition Language (SDL)</h2>
            <p>The complete GraphQL schema in SDL format:</p>
            <div class="schema-viewer" id="schema-content">{sdl_formatted}</div>
            <button class="copy-button" onclick="copySchema()">📋 Copy Schema</button>
        </div>

        <div id="playground" class="tab-content">
            <h2>🎮 Try It Out</h2>
            <div class="info-box">
                <h3>Interactive GraphQL Clients</h3>
                <p>Use one of these interactive clients to explore the API:</p>
                <ul style="margin-left: 20px; margin-top: 10px;">
                    <li><a href="{endpoint}/playground" target="_blank">GraphQL Playground</a> - Full-featured GraphQL IDE</li>
                    <li><a href="{endpoint}/graphiql" target="_blank">GraphiQL</a> - Lightweight GraphQL IDE</li>
                </ul>
            </div>

            <h3 style="margin-top: 30px;">📚 Example Queries</h3>
            <p>Here are some example queries to get you started:</p>

            <h4 style="margin-top: 20px;">Introspection Query</h4>
            <pre style="background: #f9f9f9; padding: 15px; border-radius: 4px; overflow-x: auto;">
query IntrospectionQuery {{
  __schema {{
    queryType {{ name }}
    mutationType {{ name }}
    subscriptionType {{ name }}
    types {{
      name
      kind
      description
    }}
  }}
}}
            </pre>

            <h4 style="margin-top: 20px;">Type Information</h4>
            <pre style="background: #f9f9f9; padding: 15px; border-radius: 4px; overflow-x: auto;">
query TypeInfo {{
  __type(name: "Query") {{
    name
    kind
    fields {{
      name
      type {{
        name
        kind
      }}
      args {{
        name
        type {{
          name
          kind
        }}
      }}
    }}
  }}
}}
            </pre>
        </div>

        <footer>
            <p>Generated by <strong>Armature GraphQL</strong></p>
            <p>Powered by <a href="https://github.com/quinnjr/armature" target="_blank">Armature Framework</a></p>
        </footer>
    </div>

    <script>
        function switchTab(tabName) {{
            // Hide all tab contents
            const contents = document.querySelectorAll('.tab-content');
            contents.forEach(content => content.classList.remove('active'));

            // Deactivate all tabs
            const tabs = document.querySelectorAll('.tab');
            tabs.forEach(tab => tab.classList.remove('active'));

            // Activate selected tab and content
            document.getElementById(tabName).classList.add('active');
            event.target.classList.add('active');
        }}

        function copySchema() {{
            const schemaContent = document.getElementById('schema-content').textContent;
            navigator.clipboard.writeText(schemaContent).then(() => {{
                const button = event.target;
                const originalText = button.textContent;
                button.textContent = '✅ Copied!';
                setTimeout(() => {{
                    button.textContent = originalText;
                }}, 2000);
            }});
        }}

        // Syntax highlighting for SDL
        function highlightSDL() {{
            const schemaContent = document.getElementById('schema-content');
            let html = schemaContent.textContent;

            // Re-escape HTML special characters BEFORE injecting any highlight
            // markup. `textContent` decodes the server-escaped SDL back to raw
            // `<`/`>`/`&`; without re-escaping, assigning `innerHTML` below
            // would re-parse SDL text as HTML and defeat the server-side
            // escaping (XSS if the SDL contains HTML-like text). The highlight
            // <span> tags are added AFTER this step, so they survive intact.
            html = html
                .replace(/&/g, '&amp;')
                .replace(/</g, '&lt;')
                .replace(/>/g, '&gt;');

            // Highlight keywords
            html = html.replace(/\b(type|interface|union|enum|input|scalar|schema|query|mutation|subscription|implements|extend)\b/g,
                '<span class="keyword">$1</span>');

            // Highlight types
            html = html.replace(/:\s*([A-Z][a-zA-Z0-9_]*)/g,
                ': <span class="type">$1</span>');

            // Highlight field names
            html = html.replace(/(\w+)(?=\s*:|\s*\()/g,
                '<span class="field">$1</span>');

            schemaContent.innerHTML = html;
        }}

        // Apply syntax highlighting on load
        window.addEventListener('load', highlightSDL);
    </script>
</body>
</html>"#,
        title = title,
        endpoint = endpoint,
        sdl_formatted = escape_html(&sdl)
    )
}

/// Generate a simple text-based schema documentation
pub fn generate_schema_docs_text<Query, Mutation, Subscription>(
    schema: &Schema<Query, Mutation, Subscription>,
) -> String
where
    Query: async_graphql::ObjectType + 'static,
    Mutation: async_graphql::ObjectType + 'static,
    Subscription: async_graphql::SubscriptionType + 'static,
{
    schema.sdl()
}

/// Generate JSON schema documentation
pub fn generate_schema_docs_json<Query, Mutation, Subscription>(
    schema: &Schema<Query, Mutation, Subscription>,
) -> serde_json::Value
where
    Query: async_graphql::ObjectType + 'static,
    Mutation: async_graphql::ObjectType + 'static,
    Subscription: async_graphql::SubscriptionType + 'static,
{
    serde_json::json!({
        "schema": schema.sdl(),
        "version": "2.0",
        "format": "graphql-sdl"
    })
}

/// Escape HTML special characters
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_graphql::{EmptyMutation, EmptySubscription, Schema};

    struct Query;

    #[async_graphql::Object]
    impl Query {
        async fn hello(&self) -> String {
            "Hello, World!".to_string()
        }
    }

    #[test]
    fn test_generate_schema_docs_html() {
        let schema = Schema::new(Query, EmptyMutation, EmptySubscription);
        let html = generate_schema_docs_html(&schema, "/graphql", "Test API");

        assert!(html.contains("Test API"));
        assert!(html.contains("/graphql"));
        assert!(html.contains("Schema Documentation"));
        assert!(html.contains("type Query"));
    }

    #[test]
    fn test_generate_schema_docs_text() {
        let schema = Schema::new(Query, EmptyMutation, EmptySubscription);
        let text = generate_schema_docs_text(&schema);

        assert!(text.contains("type Query"));
        assert!(text.contains("hello"));
    }

    #[test]
    fn test_generate_schema_docs_json() {
        let schema = Schema::new(Query, EmptyMutation, EmptySubscription);
        let json = generate_schema_docs_json(&schema);

        assert!(json["schema"].is_string());
        assert_eq!(json["format"], "graphql-sdl");
    }

    #[test]
    fn test_escape_html() {
        assert_eq!(escape_html("<script>"), "&lt;script&gt;");
        assert_eq!(escape_html("a & b"), "a &amp; b");
        assert_eq!(escape_html("\"quoted\""), "&quot;quoted&quot;");
    }
}
