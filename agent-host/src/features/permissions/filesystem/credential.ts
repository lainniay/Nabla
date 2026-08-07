export function isCredentialPath(path: string): boolean {
  const normalized = path.replace(/\\/gu, "/").toLocaleLowerCase();
  return [
    "/.ssh/",
    "/.aws/",
    "/.config/gcloud/",
    "/credentials",
    "/auth.json",
    "/.env",
  ].some((marker) => normalized.includes(marker));
}
