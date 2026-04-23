// Example: User API handler
// Demonstrates path parameters, query params, and CRUD operations

let method = request.method;
let user_id = request.param("id");

// Simulated database (in real use, you'd call Rust functions)
let users = #{
    "1": #{ id: 1, name: "Alice", email: "alice@example.com" },
    "2": #{ id: 2, name: "Bob", email: "bob@example.com" },
    "3": #{ id: 3, name: "Charlie", email: "charlie@example.com" }
};

if method == "GET" {
    if user_id != () {
        // Get single user
        let user = users[user_id];
        if user != () {
            response.json(user)
        } else {
            not_found().body("User not found")
        }
    } else {
        // List all users with optional filtering
        let filter = request.query("name");
        let result = [];

        for id in users.keys() {
            let user = users[id];
            if filter == () || user.name.contains(filter) {
                result.push(user);
            }
        }

        response.json(#{
            users: result,
            count: result.len()
        })
    }
} else if method == "POST" {
    // Create user
    let body = request.json();

    if body.name == () || body.email == () {
        bad_request().json(#{
            error: "Missing required fields: name, email"
        })
    } else {
        let new_id = users.len() + 1;
        let new_user = #{
            id: new_id,
            name: body.name,
            email: body.email
        };

        created()
            .header("location", `/users/${new_id}`)
            .json(new_user)
    }
} else if method == "PUT" {
    // Update user
    if user_id == () {
        bad_request().json(#{ error: "User ID required" })
    } else {
        let body = request.json();
        let updated = #{
            id: parse_int(user_id),
            name: body.name,
            email: body.email
        };
        response.json(updated)
    }
} else if method == "DELETE" {
    // Delete user
    if user_id == () {
        bad_request().json(#{ error: "User ID required" })
    } else {
        no_content()
    }
} else {
    method_not_allowed()
}

