// Example Rhai handler script
// This script handles HTTP requests and returns responses

let method = request.method;
let path = request.path;

log_info(`Handling ${method} ${path}`);

// Route based on method
if method == "GET" {
    // Return JSON response
    response.json(#{
        message: "Hello from Rhai!",
        method: method,
        path: path,
        timestamp: "2024-01-01T00:00:00Z"
    })
} else if method == "POST" {
    // Parse JSON body
    let body = request.json();

    log_info(`Received: ${to_json(body)}`);

    // Echo back with created status
    let resp = created();
    resp.json(#{
        received: body,
        status: "created"
    })
} else {
    method_not_allowed()
}

