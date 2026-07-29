## Security Policy

### Supported Versions

We release patches for security vulnerabilities. Currently supported versions:

| Version | Supported          |
| ------- | ------------------ |
| 0.1.x   | :white_check_mark: |

### Reporting a Vulnerability

**Please do not report security vulnerabilities through public GitHub issues.**

Instead, please use [GitHub Security Advisories](https://github.com/quinnjr/armature/security/advisories/new) to privately report a vulnerability.

#### What to Include

Please include the following information in your report:

- Type of vulnerability
- Full paths of source file(s) related to the vulnerability
- Location of the affected source code (tag/branch/commit or direct URL)
- Step-by-step instructions to reproduce the issue
- Proof-of-concept or exploit code (if possible)
- Impact of the vulnerability, including how an attacker might exploit it

#### Response Timeline

- **Acknowledgment**: Within 48 hours
- **Initial Assessment**: Within 7 days
- **Fix Timeline**: Depends on severity
  - Critical: Within 7 days
  - High: Within 14 days
  - Medium: Within 30 days
  - Low: Next release cycle

#### Disclosure Policy

- Security vulnerabilities are disclosed after a fix is available
- We will credit reporters unless they wish to remain anonymous
- We follow a coordinated disclosure process
- Public disclosure occurs after users have had time to update (typically 2 weeks after release)

### Security Best Practices

When using Armature in production:

1. **Keep Dependencies Updated**
   - Run `cargo audit` regularly
   - Update dependencies promptly when security patches are released
   - Use Dependabot or similar tools for automated updates

2. **Authentication & Authorization**
   - Use strong JWT secrets (minimum 256 bits for HS256)
   - Enable HTTPS/TLS for all production deployments
   - Implement proper RBAC using guards
   - Never store sensitive data in JWT payload
   - Use short token expiration times

3. **Input Validation**
   - Use the validation framework for all user inputs
   - Sanitize data before database operations
   - Validate file uploads strictly
   - Implement rate limiting on sensitive endpoints

4. **Security Headers**
   - Use `armature-security` middleware for HTTP security headers
   - Configure Content Security Policy (CSP) appropriately
   - Enable HSTS for HTTPS deployments
   - Set appropriate CORS policies

5. **Secrets Management**
   - Never commit secrets to version control
   - Use environment variables or secret management services
   - Rotate secrets regularly
   - Use different secrets for development/staging/production

6. **Database Security**
   - Use parameterized queries (Armature does this by default)
   - Apply principle of least privilege for database accounts
   - Encrypt sensitive data at rest
   - Enable database audit logging

7. **Monitoring & Logging**
   - Enable OpenTelemetry observability
   - Monitor for unusual patterns
   - Log security-relevant events
   - Set up alerts for suspicious activity
   - Do not log sensitive data (passwords, tokens, etc.)

8. **Deployment**
   - Run containers as non-root user
   - Use minimal base images
   - Scan container images for vulnerabilities
   - Keep host OS and runtime updated
   - Enable firewall rules

### Security Features

Armature provides several built-in security features:

- **JWT Authentication** (`armature-jwt`)
  - Industry-standard token-based auth
  - Multiple algorithm support (HS256, RS256, ES256)
  - Configurable expiration and validation

- **OAuth2/OIDC** (`armature-auth`)
  - Provider integrations (Google, Microsoft, AWS Cognito, Okta, Auth0)
  - PKCE support for mobile/SPA apps
  - Secure token handling

- **SAML 2.0** (`armature-auth`)
  - Enterprise SSO support
  - Service Provider implementation
  - Signature verification

- **Security Middleware** (`armature-security`)
  - Helmet-like security headers
  - CSP, HSTS, X-Frame-Options, etc.
  - Configurable per application

- **HTTPS/TLS** (`armature-core`)
  - Built-in TLS support
  - Certificate management
  - Automatic HTTP to HTTPS redirect

- **Rate Limiting** (`armature-core`)
  - Multiple algorithms (Token Bucket, Sliding Window, etc.)
  - Configurable limits per endpoint
  - Protection against abuse

- **Input Validation** (`armature-validation`)
  - 18+ built-in validators
  - Custom rule builders
  - Automatic request validation

### Known Security Limitations

- **No built-in CSRF protection**: Implement CSRF tokens in your application if needed
- **No built-in XSS protection**: Sanitize user input and output in your application
- **No automatic SQL injection protection**: Use proper ORM/query builders
- **Session management**: Armature is stateless by design; implement client-side session management with JWTs

### Security Audit

Armature has not yet undergone a professional security audit. We welcome:

- Security researchers to review our code
- Penetration testing reports
- Recommendations for security improvements

### Dependencies

We actively monitor our dependencies for security vulnerabilities using:

- `cargo-audit` in CI/CD pipeline
- GitHub Dependabot
- Dependency scanning in CI/CD

### Contact

For security-related questions or concerns, contact:

- **Security Reports**: Via [GitHub Security Advisories](https://github.com/quinnjr/armature/security/advisories/new)
- **General Questions**: Open a [GitHub Issue](https://github.com/quinnjr/armature/issues)
- **Maintainer**: Joseph Quinn

### Attribution

We would like to thank the following researchers for responsibly disclosing security issues:

<!-- Will be populated as issues are reported and fixed -->

---

**Note**: This security policy is subject to change. Check back regularly for updates.
