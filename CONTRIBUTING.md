# Contributing to Armature

Thank you for your interest in contributing to Armature! This document provides guidelines and instructions for contributing.

## Development Setup

1. Clone the repository:
```bash
git clone --recurse-submodules https://github.com/yourusername/armature.git
cd armature
# If you cloned without --recurse-submodules:
git submodule update --init --recursive
```

2. Build the project:
```bash
cargo build
```

3. Run tests:
```bash
cargo test
```

4. Run the example:
```bash
cargo run --example full_example
```

## Project Structure

The project is organized as a Cargo workspace with three main crates:

- `armature-core/` - Core runtime functionality
- `armature-macro/` - Procedural macros for decorators
- `src/` - Main library that re-exports everything
- `examples/` - Example applications
- `docs/` - Documentation

## Making Changes

1. Create a new branch for your feature:
```bash
git checkout -b feature/your-feature-name
```

2. Make your changes and ensure tests pass:
```bash
cargo test
cargo clippy -- -D warnings
cargo fmt --check
```

3. Add tests for new functionality

4. Update documentation as needed

5. Commit your changes with clear commit messages

6. Push your branch and create a pull request

## Code Style

- Follow Rust naming conventions
- Use `cargo fmt` to format code
- Use `cargo clippy` to catch common mistakes
- Write doc comments for public APIs
- Keep functions focused and small

## Testing

- Write unit tests for individual functions
- Write integration tests for API behavior
- Ensure all tests pass before submitting PR
- Aim for high test coverage

## Documentation

- Update README.md for user-facing changes
- Write doc comments for public APIs
- Include examples in doc comments
- Update relevant guides in `docs/` directory

## Pull Request Process

1. Update documentation and tests
2. Ensure CI passes
3. Request review from maintainers
4. Address review feedback
5. Maintainer will merge once approved

## Code of Conduct

- Be respectful and inclusive
- Welcome newcomers
- Focus on constructive feedback
- Follow the project's code of conduct

## Questions?

Feel free to open an issue for questions or discussions.

## License

By contributing, you agree that your contributions will be licensed under the MIT License.

