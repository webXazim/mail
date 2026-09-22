type BrandIdentityProps = {
  className?: string
  logoClassName?: string
  showName?: boolean
}

export function BrandIdentity({
  className = '',
  logoClassName = 'brand-logo',
  showName = true,
}: BrandIdentityProps) {
  return (
    <span className={`brand-identity ${className}`.trim()}>
      <img className={logoClassName} src="/cs-mail-logo.png" alt="" aria-hidden="true" />
      {showName && <span className="brand-name">CS Mail</span>}
    </span>
  )
}
