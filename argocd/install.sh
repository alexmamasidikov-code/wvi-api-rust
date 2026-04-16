#!/bin/bash
# Install ArgoCD in Kubernetes cluster
kubectl create namespace argocd 2>/dev/null
kubectl apply -n argocd -f https://raw.githubusercontent.com/argoproj/argo-cd/stable/manifests/install.yaml
echo "Waiting for ArgoCD to be ready..."
kubectl wait --for=condition=available deployment/argocd-server -n argocd --timeout=300s
echo "ArgoCD installed. Get initial password:"
echo "kubectl -n argocd get secret argocd-initial-admin-secret -o jsonpath='{.data.password}' | base64 -d"
echo ""
echo "Apply WVI API application:"
echo "kubectl apply -f argocd/application.yaml"
