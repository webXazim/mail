import { ServiceBrandSwitcher } from './navigation/ServiceBrandSwitcher'

type BrandIdentityProps = {
  className?: string
  logoClassName?: string
  showName?: boolean
}

export function BrandIdentity({
  className = '',
  showName = true,
}: BrandIdentityProps) {
  return (
    <div className={`brand-identity ${className}`.trim()}>
      <ServiceBrandSwitcher activeService="mail" compact={!showName} />
    </div>
  )
}
