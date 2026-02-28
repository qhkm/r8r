resource "aws_ecs_cluster" "main" {
  name = "${var.project_name}-cluster"

  setting {
    name  = "containerInsights"
    value = "enabled"
  }

  tags = var.tags
}

resource "aws_security_group" "task" {
  name        = "${var.project_name}-task-sg"
  description = "Allow ALB to reach r8r tasks"
  vpc_id      = aws_vpc.main.id

  ingress {
    description     = "HTTP from ALB"
    from_port       = 8080
    to_port         = 8080
    protocol        = "tcp"
    security_groups = [aws_security_group.alb.id]
  }

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }

  tags = var.tags
}

resource "aws_security_group" "efs" {
  name        = "${var.project_name}-efs-sg"
  description = "Allow ECS tasks to mount EFS"
  vpc_id      = aws_vpc.main.id

  ingress {
    description     = "NFS from ECS tasks"
    from_port       = 2049
    to_port         = 2049
    protocol        = "tcp"
    security_groups = [aws_security_group.task.id]
  }

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }

  tags = var.tags
}

resource "aws_efs_file_system" "shared" {
  encrypted = true

  tags = merge(var.tags, {
    Name = "${var.project_name}-shared-efs"
  })
}

resource "aws_efs_mount_target" "private" {
  for_each = aws_subnet.private

  file_system_id  = aws_efs_file_system.shared.id
  subnet_id       = each.value.id
  security_groups = [aws_security_group.efs.id]
}
