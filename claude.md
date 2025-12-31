# Development Notes

## Pre-commit Checklist

Before committing code, always run:

1. **Tests**: `cargo test` - Ensure all tests pass
2. **Clippy**: `cargo clippy` - Check for common mistakes and style issues
3. **Format**: `cargo fmt` - Format code according to Rust standards
4. **Build**: `cargo build` - Verify the project compiles

This helps catch issues early and maintains code quality.

## Testing Guidelines

Maintain comprehensive test coverage:

- **Unit Tests**: Test individual functions in isolation
- **Integration Tests**: Test HTTP endpoints through the web framework
- **Edge Cases**: Test error conditions, invalid inputs, and boundary cases

Aim for high coverage across all code paths. Tests should verify both success and failure scenarios.
