import type { SchemaJson } from '@cedar-policy/cedar-wasm/web'

type Attribute = { type: string; required?: boolean; element?: unknown; attributes?: Record<string, unknown> }

const string: Attribute = { type: 'String' }
const bool: Attribute = { type: 'Boolean' }
const long: Attribute = { type: 'Long' }
const stringSet: Attribute = { type: 'Set', element: { type: 'String' } }

const CONTEXT = {
  type: 'Record',
  attributes: {
    agent: string,
    delegated: bool,
    arguments: { type: 'Record', attributes: {} },
    time: {
      type: 'Record',
      attributes: { weekday: long, hour: long },
    },
  },
}

function action(resourceType: string) {
  return {
    appliesTo: {
      principalTypes: ['User', 'Agent'],
      resourceTypes: [resourceType],
      context: CONTEXT,
    },
  }
}

export const AGENT_SCHEMA = {
  FerrisKey: {
    entityTypes: {
      User: {
        memberOfTypes: ['Group'],
        shape: {
          type: 'Record',
          attributes: {
            roles: stringSet,
            groups: stringSet,
            organisation: { ...string, required: false },
          },
        },
      },
      Group: { shape: { type: 'Record', attributes: {} } },
      Agent: { shape: { type: 'Record', attributes: {} } },
      McpServer: { shape: { type: 'Record', attributes: {} } },
      Tool: {
        memberOfTypes: ['McpServer'],
        shape: {
          type: 'Record',
          attributes: {
            server: string,
            tool: string,
            annotations: {
              type: 'Record',
              attributes: {
                readOnlyHint: { ...bool, required: false },
                destructiveHint: { ...bool, required: false },
              },
            },
          },
        },
      },
      Resource: {
        memberOfTypes: ['McpServer'],
        shape: { type: 'Record', attributes: { server: string } },
      },
      Prompt: {
        memberOfTypes: ['McpServer'],
        shape: { type: 'Record', attributes: { server: string } },
      },
    },
    actions: {
      call_tool: action('Tool'),
      read_resource: action('Resource'),
      get_prompt: action('Prompt'),
    },
  },
} as unknown as SchemaJson<string>
