/**
 * JSON Schema definitions and Rust ToolDefinition shapes for coding-agent tools.
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

// --- Tool schemas ---

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

// --- Tool definitions for the model ---

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

/** Tool names for reference. */
export const TOOL_NAMES = {
  readFile: "read_file",
  listFiles: "list_files",
  searchFiles: "search_files",
  writeFile: "write_file",
} as const;
