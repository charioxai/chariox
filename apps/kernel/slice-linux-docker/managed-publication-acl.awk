BEGIN {
  RS = ""
  FS = "\n"
}

function owner_permissions(value) {
  return value ~ /^[r-][w-][x-]$/
}

function fail() {
  invalid = 1
}

{
  owner = chariox = chariox_exec = docker = mapped = mapped_exec = 0
  owning_group = mask = mask_exec = mask_traversal = other = 0
  default_owner = default_chariox = default_mapped = 0
  default_group = default_mask = default_other = defaults = 0
  for (field = 1; field <= NF; field++) {
    line = $field
    if (line ~ /^user::/ && owner_permissions(substr(line, 7))) owner++
    else if (line == "user:" chariox_uid ":rw-") chariox++
    else if (line == "user:" chariox_uid ":rwx") { chariox++; chariox_exec++ }
    else if (line == "user:" docker_uid ":--x") docker++
    else if (line == "user:" mapped_slice_uid ":rw-") mapped++
    else if (line == "user:" mapped_slice_uid ":rwx") { mapped++; mapped_exec++ }
    else if (line == "group::---" || (mode == "traversal" && line == "group::--x")) owning_group++
    else if (line == "mask::rw-") mask++
    else if (line == "mask::rwx") { mask++; mask_exec++ }
    else if (line == "mask::--x") { mask++; mask_traversal++ }
    else if (line == "other::---") other++
    else if (line == "default:user::rwx") { default_owner++; defaults++ }
    else if (line == "default:user:" chariox_uid ":rwx") { default_chariox++; defaults++ }
    else if (line == "default:user:" mapped_slice_uid ":rwx") { default_mapped++; defaults++ }
    else if (line == "default:group::---") { default_group++; defaults++ }
    else if (line == "default:mask::rwx") { default_mask++; defaults++ }
    else if (line == "default:other::---") { default_other++; defaults++ }
    else fail()
  }
  if (owner != 1 || owning_group != 1 || mask != 1 || other != 1) fail()
  if (mode == "traversal") {
    if (docker != 1 || chariox != 0 || mapped != 0 || mask_traversal != 1 || defaults != 0) fail()
  } else if (mode == "repository") {
    if (docker != 1 || chariox_exec != 1 || mapped_exec != 1 || mask_exec != 1) fail()
    if (default_owner != 1 || default_chariox != 1 || default_mapped != 1 ||
        default_group != 1 || default_mask != 1 || default_other != 1 || defaults != 6) fail()
  } else if (mode == "directory") {
    if (docker != 0 || chariox_exec != 1 || mapped_exec != 1 || mask_exec != 1) fail()
    if (default_owner != 1 || default_chariox != 1 || default_mapped != 1 ||
        default_group != 1 || default_mask != 1 || default_other != 1 || defaults != 6) fail()
  } else if (mode == "file") {
    if (docker != 0 || chariox != 1 || mapped != 1 || mask_traversal != 0 || defaults != 0) fail()
  } else fail()
  records++
}

END {
  if (invalid || (required && records != 1)) exit 1
}
