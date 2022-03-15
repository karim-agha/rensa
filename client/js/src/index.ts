const worlds = 'world';

export function hello(world: string = worlds): string {
  return `Hello ${world}! `;
}