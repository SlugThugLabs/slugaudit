"""slugaudit-mcp server package.

Cleanly separated modules for config, state, connection pooling,
auto-sync, tool definitions, handlers, and server. Deliberately does not
eagerly re-export its submodules here: app.sync (and transitively app.server)
imports from services.import_service, which imports app.manifest — eagerly
importing that whole chain the moment anything merely imports the `app`
package (e.g. `import services`, since services/import_service.py depends on
app.manifest) creates a circular import. Every consumer in this codebase
already imports the specific submodule it needs directly
(`from app.pool import ...`, `from app.config import load_config`, etc.);
`from app import <submodule>` still works via Python's standard submodule
fallback without this package needing to import anything itself.
"""
