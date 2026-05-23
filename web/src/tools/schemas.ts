/**
 * JSON Schema definitions and Rust ToolDefinition shapes for coding-agent tools.
 *
 * Two tool surfaces coexist:
 * 1. Legacy tools (read_file, write_file, list_files, search_files) — kept for
 *    backward compatibility with existing tests and the Milestone 3 integration.
 * 2. pi-compatible tools (read, write, edit, ls, grep, find, bash) — matching
 *    the ../pi default coding tool set, used by new code and Milestone 3.6+.
 *
 * These are the tool definitions that get passed to the model via
 * AgentOptions.tools and are serialized into the Rust core.
 */

// --- Rust ToolDefinition shape ---

export interface ToolDefinition {
  name: string;
  label: string;
  description: string;
  parameters: object;
  execution_mode: "parallel" | "sequential";
}

// ========================================================================
// Legacy tool schemas (Milestone 3)
// ========================================================================

export const readFileSchema: object = {
  type: "object",
  properties: {
    path: {
      type: "string",
      description: "Workspace-root-relative path of the file to read (e.g. \"src/main.rs\").",
    },
  },
  required: ["path"],
  additionalProperties: false,
};

export const listFilesSchema: object = {
  type: "object",
  properties: {
    path: {
      type: "string",
      description:
        "Workspace-root-relative directory to list. Empty string or omitted lists the workspace root.",
    },
  },
  additionalProperties: false,
};

export const searchFilesSchema: object = {
  type: "object",
  properties: {
    pattern: {
      type: "string",
      description: "Search pattern. Matches against file paths (e.g. \"*.rs\", \"test_\").",
    },
    path: {
      type: "string",
      description:
        "Workspace-root-relative directory to search in. Empty string or omitted searches from root.",
    },
  },
  required: ["pattern"],
  additionalProperties: false,
};

export const writeFileSchema: object = {
  type: "object",
  properties: {
    path: {
      type: "string",
      description: "Workspace-root-relative path of the file to write (e.g. \"src/main.rs\").",
    },
    content: {
      type: "string",
      description: "The full content to write to the file.",
    },
  },
  required: ["path", "content"],
  additionalProperties: false,
};

// ========================================================================
// pi-compatible tool schemas (Milestone 3.6)
// ========================================================================

export const readToolSchema: object = {
  type: "object",
  properties: {
    path: {
      type: "string",
      description: "Workspace-root-relative path of the file to read (e.g. \"src/main.ts\").",
    },
    offset: {
      type: "number",
      description: "Line number to start reading from (1-based). Omit to read from the beginning.",
    },
    limit: {
      type: "number",
      description: "Maximum number of lines to return. Omit to read all lines.",
    },
  },
  required: ["path"],
  additionalProperties: false,
};

export const writeToolSchema: object = {
  type: "object",
  properties: {
    path: {
      type: "string",
      description: "Workspace-root-relative path of the file to write.",
    },
    content: {
      type: "string",
      description: "The full content to write to the file.",
    },
  },
  required: ["path", "content"],
  additionalProperties: false,
};

export const editToolSchema: object = {
  type: "object",
  properties: {
    path: {
      type: "string",
      description: "Workspace-root-relative path of the file to edit.",
    },
    edits: {
      type: "array",
      items: {
        type: "object",
        properties: {
          oldText: { type: "string", description: "Exact text to find in the file." },
          newText: { type: "string", description: "Replacement text." },
        },
        required: ["oldText", "newText"],
        additionalProperties: false,
      },
      description: "Array of exact-find-and-replace edits to apply sequentially.",
    },
  },
  required: ["path", "edits"],
  additionalProperties: false,
};

export const lsToolSchema: object = {
  type: "object",
  properties: {
    path: {
      type: "string",
      description:
        "Workspace-root-relative directory to list. Omit or empty string lists the workspace root.",
    },
    limit: {
      type: "number",
      description: "Maximum number of entries to return.",
    },
  },
  additionalProperties: false,
};

export const grepToolSchema: object = {
  type: "object",
  properties: {
    pattern: {
      type: "string",
      description: "Search pattern (regex by default, or literal if literal is true).",
    },
    path: {
      type: "string",
      description:
        "Workspace-root-relative directory or file to search in. Omit to search from root.",
    },
    glob: {
      type: "string",
      description: "File glob to filter (e.g. \"*.ts\"). Only matching files are searched.",
    },
    ignoreCase: {
      type: "boolean",
      description: "If true, perform case-insensitive search.",
    },
    literal: {
      type: "boolean",
      description: "If true, treat pattern as a literal string instead of regex.",
    },
    context: {
      type: "number",
      description: "Number of context lines before and after each match.",
    },
    limit: {
      type: "number",
      description: "Maximum number of matches to return.",
    },
  },
  required: ["pattern"],
  additionalProperties: false,
};

export const findToolSchema: object = {
  type: "object",
  properties: {
    pattern: {
      type: "string",
      description: "Glob-like pattern to match against file paths (e.g. \"*.ts\", \"src/**/*.rs\").",
    },
    path: {
      type: "string",
      description: "Workspace-root-relative directory to search in. Omit to search from root.",
    },
    limit: {
      type: "number",
      description: "Maximum number of results to return.",
    },
  },
  required: ["pattern"],
  additionalProperties: false,
};

export const bashToolSchema: object = {
  type: "object",
  properties: {
    command: {
      type: "string",
      description: "Command to execute. Only a small whitelist of test commands is supported.",
    },
  },
  required: ["command"],
  additionalProperties: false,
};

// ========================================================================
// Legacy tool definitions (Milestone 3)
// ========================================================================

export const CODING_TOOLS: ToolDefinition[] = [
  {
    name: "read_file",
    label: "Read File",
    description:
      "Read the contents of a file in the workspace. " +
      "Returns the full file content as text. " +
      "Use workspace-root-relative paths (e.g. \"src/main.rs\"). " +
      "Fails if the file does not exist.",
    parameters: readFileSchema,
    execution_mode: "parallel",
  },
  {
    name: "list_files",
    label: "List Files",
    description:
      "List files and directories in a workspace directory. " +
      "Returns each entry on its own line with a trailing / for directories. " +
      "If path is omitted or empty, lists the workspace root.",
    parameters: listFilesSchema,
    execution_mode: "parallel",
  },
  {
    name: "search_files",
    label: "Search Files",
    description:
      "Search for files by name pattern. " +
      "Returns all matching workspace-root-relative paths, one per line. " +
      "The pattern matches against the full relative path (e.g. \"*.rs\" matches \"src/main.rs\").",
    parameters: searchFilesSchema,
    execution_mode: "parallel",
  },
  {
    name: "write_file",
    label: "Write File",
    description:
      "Write content to a file in the workspace. Creates parent directories if needed. " +
      "Overwrites the file if it already exists. " +
      "Use workspace-root-relative paths.",
    parameters: writeFileSchema,
    execution_mode: "sequential",
  },
];

// ========================================================================
// pi-compatible tool definitions (Milestone 3.6)
// ========================================================================

const READ_TOOL: ToolDefinition = {
  name: "read",
  label: "Read",
  description:
    "Read the contents of a file in the workspace. Returns file content as text. " +
    "Supports optional line-based offset and limit for reading portions of large files. " +
    "Use workspace-root-relative paths. Fails if the file does not exist.",
  parameters: readToolSchema,
  execution_mode: "parallel",
};

const WRITE_TOOL: ToolDefinition = {
  name: "write",
  label: "Write",
  description:
    "Write content to a file in the workspace. Creates parent directories if needed. " +
    "Overwrites the file if it already exists. Use workspace-root-relative paths.",
  parameters: writeToolSchema,
  execution_mode: "sequential",
};

const EDIT_TOOL: ToolDefinition = {
  name: "edit",
  label: "Edit",
  description:
    "Apply exact find-and-replace edits to a file. Each edit specifies oldText to find " +
    "and newText to replace it with. Edits are applied sequentially. " +
    "Fails if any oldText is not found. Use workspace-root-relative paths.",
  parameters: editToolSchema,
  execution_mode: "sequential",
};

const LS_TOOL: ToolDefinition = {
  name: "ls",
  label: "List Directory",
  description:
    "List files and directories in a workspace directory. Returns each entry on its own " +
    "line with a trailing / for directories. Omit path to list the workspace root.",
  parameters: lsToolSchema,
  execution_mode: "parallel",
};

const GREP_TOOL: ToolDefinition = {
  name: "grep",
  label: "Grep",
  description:
    "Search file contents for a pattern. Returns matching lines with file paths. " +
    "Supports regex or literal matching, case sensitivity control, glob filtering, " +
    "and context lines around matches.",
  parameters: grepToolSchema,
  execution_mode: "parallel",
};

const FIND_TOOL: ToolDefinition = {
  name: "find",
  label: "Find Files",
  description:
    "Find files by name pattern. Returns matching workspace-root-relative paths. " +
    "Supports glob-like patterns (e.g. \"*.ts\", \"src/**/*.rs\").",
  parameters: findToolSchema,
  execution_mode: "parallel",
};

const BASH_TOOL: ToolDefinition = {
  name: "bash",
  label: "Bash",
  description:
    "Execute a shell command. Only a constrained whitelist of deterministic test commands " +
    "is supported. No real filesystem or network access. Returns stdout, stderr, and exit code.",
  parameters: bashToolSchema,
  execution_mode: "sequential",
};

// --- Tool groups ---

/** Default coding tool set: read, bash, edit, write */
export const PI_CODING_TOOLS: ToolDefinition[] = [READ_TOOL, BASH_TOOL, EDIT_TOOL, WRITE_TOOL];

/** Read-only tool set: read, grep, find, ls */
export const PI_READ_ONLY_TOOLS: ToolDefinition[] = [READ_TOOL, GREP_TOOL, FIND_TOOL, LS_TOOL];

/** All tools: read, bash, edit, write, grep, find, ls */
export const PI_ALL_TOOLS: ToolDefinition[] = [
  READ_TOOL,
  BASH_TOOL,
  EDIT_TOOL,
  WRITE_TOOL,
  GREP_TOOL,
  FIND_TOOL,
  LS_TOOL,
];

// --- Tool name constants ---

/** Legacy tool names (Milestone 3) */
export const TOOL_NAMES = {
  readFile: "read_file",
  listFiles: "list_files",
  searchFiles: "search_files",
  writeFile: "write_file",
} as const;

/** pi-compatible tool names (Milestone 3.6) */
export const PI_TOOL_NAMES = {
  read: "read",
  write: "write",
  edit: "edit",
  ls: "ls",
  grep: "grep",
  find: "find",
  bash: "bash",
} as const;
