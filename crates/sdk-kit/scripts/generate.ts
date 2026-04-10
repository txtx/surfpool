/**
 * Post-generation fixup for ts-rs generated bindings.
 *
 * ts-rs doesn't add imports for types referenced in `ts(type = "...")`
 * overrides. This script patches the generated files to add missing imports.
 */
import { readFileSync, writeFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

const GENERATED_DIR = join(import.meta.dirname, '..', 'src', 'generated');

// Map of type names to their file (without extension)
const typeFiles = new Map<string, string>();
for (const file of readdirSync(GENERATED_DIR)) {
    if (file.endsWith('.ts')) {
        typeFiles.set(file.replace('.ts', ''), file);
    }
}

// For each generated file, find type references that need imports
for (const file of readdirSync(GENERATED_DIR)) {
    if (!file.endsWith('.ts')) continue;

    const filePath = join(GENERATED_DIR, file);
    let content = readFileSync(filePath, 'utf-8');
    const typeName = file.replace('.ts', '');

    // Find all type names referenced in the file body (after imports)
    const existingImports = new Set<string>();
    for (const match of content.matchAll(/import type \{ (\w+) \}/g)) {
        existingImports.add(match[1]!);
    }

    // Check for type references that aren't imported and aren't the file's own type
    const missingImports: string[] = [];
    for (const [name, _sourceFile] of typeFiles) {
        if (name === typeName) continue;
        if (existingImports.has(name)) continue;

        // Check if this type name appears in the body (not in import lines)
        const bodyLines = content.split('\n').filter(l => !l.startsWith('import '));
        const body = bodyLines.join('\n');
        // Match whole word only (not part of another identifier)
        const regex = new RegExp(`\\b${name}\\b`);
        if (regex.test(body)) {
            missingImports.push(name);
        }
    }

    if (missingImports.length > 0) {
        const importLines = missingImports
            .map(name => `import type { ${name} } from "./${name}";`)
            .join('\n');

        // Insert after the ts-rs header comment
        const headerEnd = content.indexOf('\n\n');
        if (headerEnd !== -1) {
            content = content.slice(0, headerEnd + 1) + importLines + '\n' + content.slice(headerEnd + 1);
        }

        writeFileSync(filePath, content);
        console.log(`Patched ${file}: added imports for ${missingImports.join(', ')}`);
    }
}
